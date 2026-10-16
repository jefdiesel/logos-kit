// Integration-confidence: lossless JSON and decoding on responses recorded
// from a LEZ 0.3 sequencer (test/fixtures; `*.synthetic.json` are the same
// shapes with u128::MAX, which a fresh chain never produces).
import { readdirSync, readFileSync } from 'node:fs'
import {
  fromHex,
  messageHash,
  nativeTransfer,
  testimonialPost,
  toBase64,
  transactionHash,
} from '@logos-kit/codec'
import { verifyHash } from '@logos-kit/codec/sign'
import { LezError } from '@logos-kit/protocol'
import { describe, expect, it } from 'vitest'
import {
  basecampModule,
  createClient,
  custom,
  decodeAccount,
  decodeBlock,
  decodeTransactionResult,
  http,
  nativeBalance,
  nodeActions,
  parseJson,
  poll,
  RpcError,
  TimeoutError,
  walletActions,
} from '../src/index.ts'
import { localAccount, sendLocalCall } from '../src/local.ts'

const dir = new URL('./fixtures/', import.meta.url)
const raw = (f: string) => readFileSync(new URL(f, dir), 'utf8')
// biome-ignore lint/suspicious/noExplicitAny: fixture JSON
const result = (f: string): any => (parseJson(raw(f)) as { result: unknown }).result
const U128_MAX = '340282366920938463463374607431768211455'
const TX = '5e83f6cba0a4ded1f8a23ee08e16828f9373cb9fce4287094ee33d732e6ef3c3'

describe('lossless JSON', () => {
  it('agrees with JSON.parse on every recorded fixture', () => {
    for (const f of readdirSync(dir).filter(
      (f) => f.endsWith('.json') && !f.includes('synthetic'),
    )) {
      expect(parseJson(raw(f)), f).toEqual(JSON.parse(raw(f)))
    }
  })
  it('keeps u128 values that JSON.parse would round', () => {
    expect(result('getAccountBalance.u128max.synthetic.json')).toBe(U128_MAX)
    expect(JSON.parse(raw('getAccountBalance.u128max.synthetic.json')).result).not.toBe(U128_MAX)
    const acct = decodeAccount(result('getAccount.u128max.synthetic.json'))
    expect(acct.nonce).toBe(U128_MAX)
    expect(nativeBalance(acct)).toBe(U128_MAX)
  })
  it('handles escapes, nesting, and refuses junk', () => {
    expect(parseJson('{"a":"x\\n\\u00e9","b":[1,-2.5e3,{"c":null}],"__proto__":1}')).toEqual(
      JSON.parse('{"a":"x\\n\\u00e9","b":[1,-2.5e3,{"c":null}],"__proto__":1}'),
    )
    expect(({} as { polluted?: number }).polluted).toBeUndefined()
    expect(() => parseJson('{"a":1} x')).toThrow()
    expect(() => parseJson('[1,]')).toThrow()
  })
})

describe('recorded responses decode', () => {
  it('accounts and balances', () => {
    expect(nativeBalance(decodeAccount(result('getAccount.json')))).toBe('1234567')
    expect(nativeBalance(decodeAccount(result('getAccountView.json')))).toBe(
      String(result('getAccountBalance.json')),
    )
    expect(decodeAccount(result('getAccount.genesis.json')).nonce).toBe('1')
  })
  it('the transaction hashes back to the hash the wallet got', () => {
    const t = decodeTransactionResult(result('getTransaction.json'))
    expect(t?.kind).toBe('public')
    if (t?.kind !== 'public') return
    expect(transactionHash(t.transaction)).toBe(TX)
    const w = t.transaction.witnesses[0]
    expect(w && verifyHash(w.signature, messageHash(t.transaction.message), w.publicKey)).toBe(true)
    expect(decodeTransactionResult(result('getTransaction.missing.json'))).toBeNull()
  })
  it('blocks: header and public transactions', () => {
    const b = decodeBlock(result('getBlock.json'))
    const t = decodeTransactionResult(result('getTransaction.json'))
    expect(b.header.id).toBe(String(result('getTransaction.json')[1]))
    expect(b.header.timestamp).toBeGreaterThan(1.7e12)
    expect(b.transactions?.map(transactionHash)).toContain(TX)
    expect(t?.kind === 'public' && b.transactions?.length).toBe(b.transactionCount)
  })
})

function replay() {
  const map: Record<string, string> = {
    getLastBlockId: 'getLastBlockId.json',
    getAccountBalance: 'getAccountBalance.json',
    getFeeState: 'getFeeState.json',
    getTransaction: 'getTransaction.json',
    getAccountsNonces: 'getAccountsNonces.json',
  }
  const seen: { method: string; params: unknown }[] = []
  const transport = custom({
    async request({ method, params }) {
      seen.push({ method, params })
      if (method === 'getAccountsNonces') {
        // The fixture answers two accounts; the node answers as many as asked.
        const all = (parseJson(raw('getAccountsNonces.json')) as { result: unknown[] }).result
        return all.slice(0, ((params as unknown[])[0] as unknown[]).length)
      }
      if (method === 'sendTransaction') {
        const tx = decodeTransactionResult([(params as string[])[0], 0])
        return tx?.kind === 'public' ? transactionHash(tx.transaction) : TX
      }
      const f = map[method]
      if (!f) throw new RpcError(-32601, 'Method not found')
      return (parseJson(raw(f)) as { result: unknown }).result
    },
  })
  return { client: createClient({ transport, chain: 'lez:local' }).extend(nodeActions), seen }
}

describe('node actions over a transport', () => {
  it('reads and waits', async () => {
    const { client } = replay()
    expect(await client.getBlockNumber()).toBe('3')
    expect(await client.getBalance('J2ZpoZWRdsnxfw6UQApFbRE87gifM4S9EBvyXaSUZsiJ')).toBe('1234567')
    expect((await client.getFeeState()).maxGasExec).toBe('10000000')
    expect((await client.waitForTransaction(TX, { interval: 1 })).kind).toBe('public')
    await expect(client.getBalance('not-an-account')).rejects.toThrow()
  })
  it('a local account signs exactly what it sends', async () => {
    const { client, seen } = replay()
    const key = fromHex('7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb')
    const acct = localAccount(key)
    const hash = await sendLocalCall(client, acct, nativeTransfer(acct.address, acct.address, '5'))
    expect(hash).toMatch(/^[0-9a-f]{64}$/)
    const param = seen.find((s) => s.method === 'sendTransaction')?.params as string[]
    const tx = decodeTransactionResult([param[0], 0])
    expect(tx?.kind).toBe('public')
    if (tx?.kind !== 'public') return
    expect(tx.transaction.message.nonces).toEqual(['1'])
    expect(tx.transaction.message.fee?.maxFee).toBe('134400000')
  })
})

describe('Basecamp module transport + wallet actions', () => {
  it('unwraps values, resolves the testimonial page, maps errors to LezError', async () => {
    const calls: unknown[][] = []
    const callModuleAsync = (
      m: string,
      method: string,
      args: unknown[],
      cb: (p: string) => void,
    ) => {
      calls.push([m, method, args])
      if (method === 'lez_getTransactionStatus')
        cb('{"value":{"handle":"h","lifecycle":"included","outcome":"success"}}')
      else if (method === 'lez_signAndSendTransaction') cb('{"value":{"handle":"h"}}')
      else if (method === 'lez_readAccount') cb('{"value":{"nonce":"0","data":""}}')
      else if (method === 'lez_getAccounts') cb('{"value":"[not json, a label]"}')
      else if (method === 'lez_signIn') cb('{"error":"cancelled"}')
      else cb('{"error":{"code":4001,"message":"Request declined"}}')
    }
    const w = createClient({
      transport: basecampModule({ callModuleAsync }),
      chain: 'lez:testnet',
    }).extend(walletActions)
    const author = 'J2ZpoZWRdsnxfw6UQApFbRE87gifM4S9EBvyXaSUZsiJ'
    const program = 'cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN'
    expect(
      await w.postTestimonial({ program, author, text: 'I use Logos Kit', timestampMs: 1 }),
    ).toEqual({ handle: 'h' })
    expect(calls.map((c) => c[1])).toEqual(['lez_readAccount', 'lez_signAndSendTransaction'])
    const sent = JSON.parse(((calls[1] as unknown[])[2] as string[])[0] as string)
    expect((calls[1] as unknown[])[0]).toBe('logos_kit_wallet')
    expect(sent.account).toBe(author)
    expect(sent.instructions[0].data).toBe(
      toBase64(testimonialPost({ program, author, text: 'I use Logos Kit', timestampMs: 1 }).data),
    )
    expect(sent.instructions[0].accounts.map((a: { writable: boolean }) => a.writable)).toEqual([
      false,
      true,
      true,
    ])
    // A string value is a string, never re-parsed as JSON.
    expect(await w.getAccounts()).toBe('[not json, a label]')
    expect((await w.waitForTransactionStatus('h', { interval: 1 })).outcome).toBe('success')
    const err = await w.connect({ chains: ['lez:testnet'] }).catch((e) => e)
    expect(err).toBeInstanceOf(LezError)
    expect(err.code).toBe(4001)
    const cancelled = await w
      .signIn({ domain: 'd', uri: 'u', nonce: 'n', issuedAt: 'i' } as never)
      .catch((e) => e)
    expect(cancelled.code).toBe(4001)
  })
  it('a bridge that never answers times out', async () => {
    const t = basecampModule({ callModuleAsync: () => {}, timeout: 1 })
    await expect(t.request('lez_getAccounts', {})).rejects.toBeInstanceOf(TimeoutError)
  })
  it('poll: hidden pauses the clock, stop ends it, visible times out', async () => {
    let calls = 0
    let hidden = 0
    const stopped = await poll(
      async () => {
        calls++
        return undefined
      },
      'never',
      { timeout: 5, interval: 2, isVisible: () => false, shouldStop: () => ++hidden > 10 },
    )
    expect(stopped).toBeUndefined()
    expect(calls).toBe(0)
    await expect(
      poll(async () => undefined, 'never', { timeout: 20, interval: 5 }),
    ).rejects.toBeInstanceOf(TimeoutError)
  })
  it('http: sendTransaction is never retried; malformed JSON is refused', async () => {
    let n = 0
    const failing = (async () => {
      n++
      throw new Error('network down')
    }) as unknown as typeof fetch
    const t = http('http://x', { fetch: failing, retryCount: 3 })
    await expect(t.request('sendTransaction', [])).rejects.toThrow('network down')
    expect(n).toBe(1)
    await expect(t.request('getLastBlockId', [])).rejects.toThrow('network down')
    expect(n).toBe(5)
    expect(() => parseJson('01')).toThrow()
    expect(() => parseJson('1.')).toThrow()
    // An escaped control character is fine; a raw one is not JSON.
    expect(parseJson(String.raw`"a\u0001"`)).toBe('a\u0001')
    expect(() => parseJson('"tab\there"')).toThrow(/control character/)
  })
})
