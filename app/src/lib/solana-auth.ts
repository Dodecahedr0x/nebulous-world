import nacl from "tweetnacl";
import bs58 from "bs58";
import { PublicKey } from "@solana/web3.js";

// Sign-In-With-Solana (SIWS-style) authentication.
//
// The client asks the server for a nonce, has the wallet sign a human-readable
// message embedding that nonce, and sends back the signature. The server
// verifies the signature against the claimed public key. No private key ever
// leaves the wallet and the nonce prevents replay. This is what gives the
// session cookie (and, downstream, the rate limiter in lib/api.ts's
// requireRateLimit) a wallet identity that actually costs something to fake —
// unlike a bare client-asserted wallet string.

const APP_NAME = "nebulous.world";

export interface AuthChallenge {
  nonce: string;
  issuedAt: string;
  statement: string;
}

/** Build the exact message string the wallet is asked to sign. */
export function buildSignInMessage(challenge: AuthChallenge): string {
  return [
    `${APP_NAME} wants you to sign in with your Solana account.`,
    "",
    challenge.statement,
    "",
    `Nonce: ${challenge.nonce}`,
    `Issued At: ${challenge.issuedAt}`,
  ].join("\n");
}

/**
 * Verify an ed25519 signature over `message` produced by `walletBase58`.
 * Returns true only if the signature is valid for that public key.
 */
export function verifySignature(
  walletBase58: string,
  message: string,
  signatureBase58: string,
): boolean {
  try {
    const pubkey = new PublicKey(walletBase58);
    const messageBytes = new TextEncoder().encode(message);
    const signatureBytes = bs58.decode(signatureBase58);
    return nacl.sign.detached.verify(
      messageBytes,
      signatureBytes,
      pubkey.toBytes(),
    );
  } catch {
    return false;
  }
}

/** Validate that a string is a well-formed Solana public key. */
export function isValidWallet(wallet: string): boolean {
  try {
    // eslint-disable-next-line no-new
    new PublicKey(wallet);
    return true;
  } catch {
    return false;
  }
}
