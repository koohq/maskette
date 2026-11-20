// Check a minisign signature, the format `tauri build` signs update files with and the updater checks them in.
// Both the key (`plugins.updater.pubkey`) and the `.sig` file are base64 of minisign's own text files:
//
//   untrusted comment: ...
//   <base64: algorithm (2 bytes) | key id (8) | public key (32) or signature (64)>
//   trusted comment: ...                  (signatures only)
//   <base64: signature of the line above> (signatures only)
//
// "Ed" signs the file itself; "ED" signs its BLAKE2b-512 hash, which is what Tauri writes.
import { createHash, createPublicKey, verify } from "node:crypto";

// The DER prefix that turns a raw 32-byte Ed25519 key into an SPKI public key.
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function lines(base64: string): string[] {
  return Buffer.from(base64.trim(), "base64").toString("utf8").split(/\r?\n/);
}

/** Why `signature` (the `.sig` file's contents) does not sign `file` with `publicKey`, or `null` when it does. */
export function signatureProblem(file: Buffer, signature: string, publicKey: string): string | null {
  const keyBytes = Buffer.from(lines(publicKey)[1] ?? "", "base64");
  if (keyBytes.length !== 42 || keyBytes.subarray(0, 2).toString() !== "Ed") {
    return "the public key is not a minisign key";
  }
  const [, sigLine, trustedLine, globalLine] = lines(signature);
  const sigBytes = Buffer.from(sigLine ?? "", "base64");
  if (sigBytes.length !== 74) return "the signature file is not a minisign signature";

  const algorithm = sigBytes.subarray(0, 2).toString();
  if (algorithm !== "Ed" && algorithm !== "ED") return `unknown signature algorithm ${algorithm}`;
  if (!sigBytes.subarray(2, 10).equals(keyBytes.subarray(2, 10))) {
    return "it was signed with another key (the key ids differ)";
  }
  const key = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, keyBytes.subarray(10)]),
    format: "der",
    type: "spki",
  });
  const signed = algorithm === "ED" ? createHash("blake2b512").update(file).digest() : file;
  const sig = sigBytes.subarray(10);
  if (!verify(null, signed, key, sig)) return "the signature does not match the file";

  // The trusted comment (file name, version) is signed together with the signature.
  const trusted = trustedLine?.replace(/^trusted comment: /, "") ?? "";
  const globalSig = Buffer.from(globalLine ?? "", "base64");
  if (!verify(null, Buffer.concat([sig, Buffer.from(trusted)]), key, globalSig)) {
    return "the signature's trusted comment does not match";
  }
  return null;
}
