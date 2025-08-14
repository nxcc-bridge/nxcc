/**
 * Securely derives a key for a given path using HKDF with configurable hash function.
 * @param base A strong CryptoKey with the `deriveBits` capability.
 * @param purpose A non-empty string describing the purpose of the output key. Used for top-level domain separation.
 * @param path An array of further domain separation values. If strings are provided, they will be UTF-8 encoded.
 *             Empty strings or zero-length arrays are not allowed to prevent ambiguous encodings.
 *             The path array itself may be empty if there is no path usefully associated with the key.
 * @param options Optional configuration object:
 *   - `length`: The desired length of the derived key in bytes. Defaults to the hash output size.
 *               Must be between 1 and 255 * hash_length bytes (HKDF limit).
 *   - `hash`: The hash algorithm to use. Defaults to "SHA-256". Supports "SHA-256", "SHA-384", and "SHA-512".
 *   - `salt`: Optional salt value. While the purpose and path provide domain separation,
 *             a salt adds an additional layer of randomness and can help when:
 *             - Multiple keys are derived from the same base key for the same purpose/path
 *             - You need to ensure different deployments/installations produce different keys
 *             - You want to add time-based or session-based uniqueness
 *
 *             To generate a salt: `crypto.getRandomValues(new Uint8Array(16))` (16-32 bytes recommended)
 *             To reuse: Store the salt alongside any data encrypted with the derived key.
 *                      The same salt must be used to re-derive the same key.
 * @returns A Promise that resolves to the derived key as a Uint8Array.
 * @throws {Error} If inputs are invalid or key derivation fails.
 */
export async function deriveKey(
  base: CryptoKey,
  purpose: string,
  path: Array<string | Uint8Array>,
  options?: {
    length?: number;
    hash?: "SHA-256" | "SHA-384" | "SHA-512";
    salt?: Uint8Array;
  },
): Promise<Uint8Array> {
  // Configure hash algorithm and related parameters
  const hashAlgorithm = options?.hash ?? "SHA-256";
  const hashOutputSizes: Record<string, number> = {
    "SHA-256": 32,
    "SHA-384": 48,
    "SHA-512": 64,
  };

  const hashOutputSize = hashOutputSizes[hashAlgorithm];
  if (!hashOutputSize) {
    throw new Error(
      `Unsupported hash algorithm: ${hashAlgorithm}. Supported: SHA-256, SHA-384, SHA-512`,
    );
  }

  // Calculate HKDF maximum output for this hash function
  const hkdfMaxOutput = 255 * hashOutputSize;
  const length = options?.length ?? hashOutputSize;

  // Input validation
  if (!base) {
    throw new Error("Base key is required");
  }

  // Runtime type check for CryptoKey
  if (!(base instanceof CryptoKey)) {
    throw new Error("Base must be a CryptoKey instance");
  }

  // Warn if key doesn't appear to be suitable for HKDF
  if (base.type !== "secret") {
    console.warn('Base key type is not "secret", may not be suitable for HKDF operations');
  }

  if (typeof purpose !== "string" || !purpose.trim()) {
    throw new Error("Purpose must be a non-empty string");
  }

  if (!Number.isInteger(length) || length <= 0 || length > hkdfMaxOutput) {
    throw new Error(
      `Invalid length: ${length}. Must be an integer between 1-${hkdfMaxOutput} bytes (HKDF-${hashAlgorithm} limit)`,
    );
  }

  if (!Array.isArray(path)) {
    throw new Error("Path must be an array");
  }

  // Validate salt if provided
  if (options?.salt !== undefined && !(options.salt instanceof Uint8Array)) {
    throw new Error("Salt must be a Uint8Array");
  }

  const salt = options?.salt ?? new Uint8Array(); // Default to empty salt if not provided

  const textEncoder = new TextEncoder();
  const MAX_INFO_SIZE = 1024 * 1024; // 1MB limit to prevent memory exhaustion

  // Helper function to convert a number to a 4-byte Big Endian Uint8Array (length prefix)
  function u32be(n: number): Uint8Array {
    if (n < 0 || n > 0xffffffff || !Number.isInteger(n)) {
      throw new Error(`Invalid length: ${n}. Must be a non-negative integer ≤ 2^32-1`);
    }
    const buffer = new ArrayBuffer(4);
    const view = new DataView(buffer);
    view.setUint32(0, n, false); // false for big-endian
    return new Uint8Array(buffer);
  }

  const infoParts: Uint8Array[] = [];

  // 1. Process purpose: Length-prefix and add to info parts
  const encodedPurpose = textEncoder.encode(purpose);
  infoParts.push(u32be(encodedPurpose.byteLength));
  infoParts.push(encodedPurpose);

  // 2. Process path items: Length-prefix each item and add to info parts
  for (let i = 0; i < path.length; i++) {
    const item = path[i];

    if (item === null || item === undefined) {
      throw new Error(`Path item at index ${i} cannot be null or undefined`);
    }

    if (typeof item !== "string" && !(item instanceof Uint8Array)) {
      throw new Error(`Path item at index ${i} must be a string or Uint8Array`);
    }

    const data: Uint8Array = typeof item === "string" ? textEncoder.encode(item) : item;

    // Disallow empty path elements to prevent ambiguous encodings
    if (data.byteLength === 0) {
      throw new Error(`Path item at index ${i} cannot be empty`);
    }

    // Validate individual item size before adding length prefix
    if (data.byteLength > 0xffffffff) {
      throw new Error(`Path item at index ${i} is too large (${data.byteLength} bytes)`);
    }

    infoParts.push(u32be(data.byteLength));
    infoParts.push(data);
  }

  // 3. Concatenate all info parts into a single Uint8Array
  // Calculate the total length first
  let totalInfoLength = 0;
  for (const part of infoParts) {
    totalInfoLength += part.byteLength;

    // Check during accumulation to catch overflow early
    if (totalInfoLength > MAX_INFO_SIZE) {
      throw new Error(`Total info size exceeds maximum ${MAX_INFO_SIZE} bytes`);
    }
  }

  // Create the final info buffer and fill it
  const concatenatedInfo = new Uint8Array(totalInfoLength);
  let offset = 0;
  for (const part of infoParts) {
    concatenatedInfo.set(part, offset);
    offset += part.byteLength;
  }

  // 4. Perform the key derivation using HKDF
  try {
    // The 'base' CryptoKey is used as the Input Keying Material (IKM).
    // 'concatenatedInfo' serves as the 'info' parameter for HKDF, providing domain separation.
    // The salt parameter adds additional randomness beyond domain separation.
    const derivedBits = await crypto.subtle.deriveBits(
      {
        name: "HKDF",
        hash: hashAlgorithm,
        salt: salt as BufferSource,
        info: concatenatedInfo,
      },
      base,
      length * 8, // Convert bytes to bits
    );

    // `deriveBits` returns an ArrayBuffer, so convert it to Uint8Array.
    return new Uint8Array(derivedBits);
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    throw new Error(`Key derivation failed: ${errorMessage}`);
  }
}
