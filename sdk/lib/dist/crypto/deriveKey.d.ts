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
export declare function deriveKey(
  base: CryptoKey,
  purpose: string,
  path: Array<string | Uint8Array>,
  options?: {
    length?: number;
    hash?: "SHA-256" | "SHA-384" | "SHA-512";
    salt?: Uint8Array;
  },
): Promise<Uint8Array>;
//# sourceMappingURL=deriveKey.d.ts.map
