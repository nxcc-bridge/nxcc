import { crypto as sdkCrypto } from "./index";

// Helper to create a test master key
async function createTestKey(): Promise<CryptoKey> {
  const keyMaterial = await crypto.subtle.importKey(
    "raw",
    new Uint8Array(32).fill(0x42), // Deterministic key for testing
    { name: "HKDF" },
    false,
    ["deriveBits"],
  );
  return keyMaterial;
}

// Helper to convert bytes to hex
function toHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

describe("sdkCrypto.deriveKey", () => {
  let masterKey: CryptoKey;

  beforeAll(async () => {
    masterKey = await createTestKey();
  });

  describe("basic functionality", () => {
    it("should derive a key with default parameters", async () => {
      const key = await sdkCrypto.deriveKey(masterKey, "test", ["path"]);
      expect(key).toBeInstanceOf(Uint8Array);
      expect(key.length).toBe(32); // SHA-256 default
    });

    it("should derive different keys for different purposes", async () => {
      const key1 = await sdkCrypto.deriveKey(masterKey, "purpose1", ["path"]);
      const key2 = await sdkCrypto.deriveKey(masterKey, "purpose2", ["path"]);
      expect(toHex(key1)).not.toBe(toHex(key2));
    });

    it("should derive different keys for different paths", async () => {
      const key1 = await sdkCrypto.deriveKey(masterKey, "test", ["path1"]);
      const key2 = await sdkCrypto.deriveKey(masterKey, "test", ["path2"]);
      expect(toHex(key1)).not.toBe(toHex(key2));
    });

    it("should derive the same key for identical inputs", async () => {
      const key1 = await sdkCrypto.deriveKey(masterKey, "test", ["path"]);
      const key2 = await sdkCrypto.deriveKey(masterKey, "test", ["path"]);
      expect(toHex(key1)).toBe(toHex(key2));
    });
  });

  describe("path handling", () => {
    it("should work with empty path", async () => {
      const key = await sdkCrypto.deriveKey(masterKey, "test", []);
      expect(key).toBeInstanceOf(Uint8Array);
      expect(key.length).toBe(32);
    });

    it("should work with mixed string and Uint8Array paths", async () => {
      const binaryPath = new Uint8Array([1, 2, 3, 4]);
      const key = await sdkCrypto.deriveKey(masterKey, "test", ["string", binaryPath]);
      expect(key).toBeInstanceOf(Uint8Array);
    });

    it("should reject empty string in path", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "test", [""])).rejects.toThrow(
        "Path item at index 0 cannot be empty",
      );
    });

    it("should reject empty Uint8Array in path", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "test", [new Uint8Array(0)])).rejects.toThrow(
        "Path item at index 0 cannot be empty",
      );
    });

    it("should reject null/undefined in path", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "test", [null as any])).rejects.toThrow(
        "Path item at index 0 cannot be null or undefined",
      );
      await expect(sdkCrypto.deriveKey(masterKey, "test", [undefined as any])).rejects.toThrow(
        "Path item at index 0 cannot be null or undefined",
      );
    });
  });

  describe("options handling", () => {
    it("should respect custom length", async () => {
      const key16 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        length: 16,
      });
      const key64 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        length: 64,
      });
      expect(key16.length).toBe(16);
      expect(key64.length).toBe(64);
    });

    it("should work with different hash algorithms", async () => {
      const sha256 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        hash: "SHA-256",
      });
      const sha384 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        hash: "SHA-384",
      });
      const sha512 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        hash: "SHA-512",
      });

      expect(sha256.length).toBe(32);
      expect(sha384.length).toBe(48);
      expect(sha512.length).toBe(64);

      // Should all be different
      expect(toHex(sha256)).not.toBe(toHex(sha384));
      expect(toHex(sha384)).not.toBe(toHex(sha512));
    });

    it("should work with salt", async () => {
      const salt1 = new Uint8Array([1, 2, 3, 4]);
      const salt2 = new Uint8Array([5, 6, 7, 8]);

      const key1 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        salt: salt1,
      });
      const key2 = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        salt: salt2,
      });
      const keyNoSalt = await sdkCrypto.deriveKey(masterKey, "test", ["path"]);

      expect(toHex(key1)).not.toBe(toHex(key2));
      expect(toHex(key1)).not.toBe(toHex(keyNoSalt));
    });
  });

  describe("input validation", () => {
    it("should reject missing base key", async () => {
      await expect(sdkCrypto.deriveKey(null as any, "test", ["path"])).rejects.toThrow(
        "Base key is required",
      );
    });

    it("should reject non-CryptoKey base", async () => {
      await expect(sdkCrypto.deriveKey({} as any, "test", ["path"])).rejects.toThrow(
        "Base must be a CryptoKey instance",
      );
    });

    it("should reject empty purpose", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "", ["path"])).rejects.toThrow(
        "Purpose must be a non-empty string",
      );
      await expect(sdkCrypto.deriveKey(masterKey, "   ", ["path"])).rejects.toThrow(
        "Purpose must be a non-empty string",
      );
    });

    it("should reject non-array path", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "test", "not-array" as any)).rejects.toThrow(
        "Path must be an array",
      );
    });

    it("should reject invalid length", async () => {
      await expect(sdkCrypto.deriveKey(masterKey, "test", ["path"], { length: 0 })).rejects.toThrow(
        "Invalid length: 0",
      );
      await expect(
        sdkCrypto.deriveKey(masterKey, "test", ["path"], { length: -1 }),
      ).rejects.toThrow("Invalid length: -1");
      await expect(
        sdkCrypto.deriveKey(masterKey, "test", ["path"], { length: 1.5 }),
      ).rejects.toThrow("Invalid length: 1.5");
    });

    it("should reject invalid hash algorithm", async () => {
      await expect(
        sdkCrypto.deriveKey(masterKey, "test", ["path"], {
          hash: "MD5" as any,
        }),
      ).rejects.toThrow("Unsupported hash algorithm: MD5");
    });

    it("should reject invalid salt", async () => {
      await expect(
        sdkCrypto.deriveKey(masterKey, "test", ["path"], {
          salt: "not-uint8array" as any,
        }),
      ).rejects.toThrow("Salt must be a Uint8Array");
    });
  });

  describe("deterministic behavior", () => {
    it("should produce consistent results across calls", async () => {
      const results = await Promise.all([
        sdkCrypto.deriveKey(masterKey, "consistency-test", ["path1", "path2"]),
        sdkCrypto.deriveKey(masterKey, "consistency-test", ["path1", "path2"]),
        sdkCrypto.deriveKey(masterKey, "consistency-test", ["path1", "path2"]),
      ]);

      const hexResults = results.map(toHex);
      expect(hexResults[0]).toBe(hexResults[1]);
      expect(hexResults[1]).toBe(hexResults[2]);
    });

    it("should handle Unicode strings correctly", async () => {
      const key1 = await sdkCrypto.deriveKey(masterKey, "test", ["hello"]);
      const key2 = await sdkCrypto.deriveKey(masterKey, "test", ["héllo"]);
      const key3 = await sdkCrypto.deriveKey(masterKey, "test", ["🔑"]);

      expect(toHex(key1)).not.toBe(toHex(key2));
      expect(toHex(key2)).not.toBe(toHex(key3));
    });
  });

  describe("edge cases", () => {
    it("should handle very long purpose strings", async () => {
      const longPurpose = "x".repeat(1000);
      const key = await sdkCrypto.deriveKey(masterKey, longPurpose, ["path"]);
      expect(key).toBeInstanceOf(Uint8Array);
    });

    it("should handle many path elements", async () => {
      const manyPaths = Array.from({ length: 100 }, (_, i) => `path${i}`);
      const key = await sdkCrypto.deriveKey(masterKey, "test", manyPaths);
      expect(key).toBeInstanceOf(Uint8Array);
    });

    it("should handle maximum allowed key length for SHA-256", async () => {
      const maxLength = 255 * 32; // HKDF limit for SHA-256
      const key = await sdkCrypto.deriveKey(masterKey, "test", ["path"], {
        length: maxLength,
      });
      expect(key.length).toBe(maxLength);
    });

    it("should reject exceeding maximum key length", async () => {
      const tooLong = 255 * 32 + 1;
      await expect(
        sdkCrypto.deriveKey(masterKey, "test", ["path"], { length: tooLong }),
      ).rejects.toThrow("Invalid length");
    });
  });
});
