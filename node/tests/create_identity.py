#!/usr/bin/env python3
"""
Generate an Ed25519 libp2p identity and save it to <output_path>.
"""
import os, sys
from libp2p.crypto.ed25519 import create_new_key_pair  # key generator


def generate_and_save_identity(output_path: str) -> None:
    kp = create_new_key_pair()  # KeyPair
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "wb") as f:
        f.write(kp.private_key.serialize())  # ready-made protobuf
    print(f"✅  Identity saved to {output_path}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("Usage: python create_identity.py <output_path>")
    generate_and_save_identity(sys.argv[1])
