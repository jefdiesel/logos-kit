// Node responses → typed values. Balances and nonces are decimal strings.
import {
  type AccountId,
  decodeTransaction,
  fromBase64,
  NATIVE_TOKEN_PROGRAM,
  type PublicTransaction,
  Reader,
  readTransaction,
  readUle,
  toHex,
} from '@logos-kit/codec'
import { decimal } from './json.ts'

export interface Account {
  nonce: string
  /** Program account → that program's data on this account. */
  shards: Record<AccountId, Uint8Array>
}

/** A shard never exceeds LEZ's DATA_MAX_LENGTH (100 KiB). */
const MAX_SHARD = 100 * 1024

export function decodeAccount(v: unknown): Account {
  const a = v as { nonce: unknown; data?: { shards?: unknown } } | null
  if (!a || typeof a !== 'object') throw new TypeError('account: not an object')
  // No prototype: shard keys come from the node.
  const shards = Object.create(null) as Record<AccountId, Uint8Array>
  const raw = (a.data && a.data.shards) || {}
  if (typeof raw !== 'object') throw new TypeError('account: shards is not an object')
  for (const k of Object.keys(raw)) {
    const bytes = (raw as Record<string, unknown>)[k]
    if (!Array.isArray(bytes) || bytes.length > MAX_SHARD)
      throw new TypeError(`account: shard ${k} is not a byte array`)
    for (const x of bytes) {
      if (typeof x !== 'number' || !Number.isInteger(x) || x < 0 || x > 255)
        throw new TypeError(`account: shard ${k} holds a non-byte`)
    }
    shards[k] = new Uint8Array(bytes)
  }
  return { nonce: decimal(a.nonce), shards }
}

/** Native balance held in an account's native-token shard (u128 LE). */
export function nativeBalance(account: Account): string {
  const s = account.shards[NATIVE_TOKEN_PROGRAM]
  return s && s.length ? readUle(s.subarray(0, 16)) : '0'
}

export interface BlockHeader {
  id: string
  hash: string
  prevHash: string
  /** Unix ms. */
  timestamp: number
}

export interface Block {
  header: BlockHeader
  /** Public transactions, when every transaction in the block is public. */
  transactions?: PublicTransaction[]
  /** Number of transactions (always known). */
  transactionCount: number
  /** The borsh block, base64, for callers that need the rest. */
  raw: string
}

export function decodeBlock(b64: string): Block {
  const bytes = fromBase64(b64)
  const r = new Reader(bytes)
  const id = r.u64()
  const prevHash = toHex(r.fixed(32))
  const hash = toHex(r.fixed(32))
  const timestamp = Number(r.u64())
  // LEZ v0.3.1 dropped the header's producer key and signature.
  const count = r.u32()
  const header = { id, hash, prevHash, timestamp }
  // A private transaction has no length prefix: decoding stops at the first
  // one. Any other decoding error is a real error.
  const transactions: PublicTransaction[] = []
  for (let i = 0; i < count; i++) {
    if (r.u8() !== 0) return { header, transactionCount: count, raw: b64 }
    transactions.push(readTransaction(r))
  }
  return { header, transactions, transactionCount: count, raw: b64 }
}

export type TransactionEnvelope =
  | { kind: 'public'; transaction: PublicTransaction; block: string }
  | { kind: 'private'; raw: string; block: string }

/** `getTransaction`'s `[base64 LeeTransaction, blockId]`. */
export function decodeTransactionResult(v: unknown): TransactionEnvelope | null {
  if (v === null || v === undefined) return null
  const [b64, block] = v as [string, unknown]
  const bytes = fromBase64(b64)
  if (bytes[0] === 0) {
    return {
      kind: 'public',
      transaction: decodeTransaction(bytes.subarray(1)),
      block: decimal(block),
    }
  }
  return { kind: 'private', raw: b64, block: decimal(block) }
}
