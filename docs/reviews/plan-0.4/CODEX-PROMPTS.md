# Codex prompts: independent research and review of the 0.4 plan (2026-10-14)

Two separate runs, one job each. Open Codex in `/Users/abu/dev/hackathon/logo-tech/logos-kit` and paste one prompt at a time, everything inside its fence. Each run writes only its own report file.

Context files both prompts rely on:
- `docs/dev/status-2026-10-14.md`
- `docs/dev/PLAN-0.4.md`
- `docs/dev/PLAN-LP0001.md`

---

## Prompt 1: "Is it already answered?"

```xml
<task>
You are an independent researcher for the Logos Kit project: a LEZ wallet, Wallet Provider SDK, NFT support and private membership proofs, entered for Logos λPrizes LP-0021 and LP-0001. The team just asked the Logos developers three questions on Discord. They want to avoid asking anything that is already answered somewhere public, and to find the answers to their open questions themselves first.

For EACH question below, search primary sources and report whether it is already answered.

Questions asked on Discord (2026-10-14):
Q1. On the reset LEZ 0.3.1 testnet (https://testnet.lez.logos.co), the old genesis test accounts and the Piñata faucet pool are empty, and logos-co/lez-faucet is pinned to LEZ v0.2.2. What is the intended way to get LGO into LEZ now? Is it a bridge deposit from the Logos blockchain ("bedrock"), and is there a faucet or EmPoWer flow on the blockchain side users can use today?
Q2. LEZ v0.3.1 removed the token, ATA and AMM programs from the builtins (PR #926), but LEZ PR #935 ("transactional actors") still has actor ports of them in its genesis. Will testnet 0.4 ship token/ATA as builtins, or will they come from logos-blockchain/lez-programs? Which program IDs should a wallet target?
Q3. Is LEZ PR #935 the base for testnet 0.4, and is it reasonable to build against it now, pinned to a commit?

Open questions from our plan (docs/dev/PLAN-0.4.md, docs/dev/status-2026-10-14.md):
Q4. Was the Logos blockchain (bedrock) testnet reset together with the LEZ zone on ~2026-10-13? Do L1 balances survive?
Q5. How does a bridge Deposit from the Logos blockchain into LEZ work today (the bridge program, CLI or wallet commands, docs)? Is there a working end-to-end example?
Q6. What is EmPoWer ("EmPoWering", proof-of-work onboarding) and is it live on the testnet? Where is its API, CLI or GUI?
Q7. Will testnet 0.4 start from a fresh genesis (all 0.3 state gone)?
Q8. What changes to the LEZ Core Module API (logos-execution-zone-module, lez_core) are planned for 0.4?
Q9. Will 0.4 expose an indexer or account-history RPC to the public, or only the sequencer (today only the sequencer is public, and its RPC sends no CORS headers)?
Q10. Does lambda-prize PR #181 (removing the LP-0021 faucet criterion) have a decision yet? Has anything else in LP-0021 or LP-0001 changed since commit ad608bf?
Q11. Is anyone proposing an "open at transaction/account" intent for the Basecamp LEZ Explorer app (logos-blockchain/lez-explorer-ui)?
Q12. Which Basecamp release is expected for 0.4, and does logos-module-builder require changes for it?
Q13. Is there any statement that 0.4 adds serial numbers or per-item metadata for NFTs, a token-holding read or assert message, or anything else that changes NFT membership-proof design?
</task>

<research_mode>
Separate observed facts, reasoned inferences, and open questions.
Search breadth first:
- GitHub orgs logos-blockchain and logos-co: issues, PRs (open and closed), PR review comments, discussions, READMEs, docs/ folders, release notes, CHANGELOGs. Especially logos-execution-zone (PRs #926, #932, #935, #949, #955, #956, #957, #974, #977 and the dev branch), lez-programs, lez-faucet, lez-explorer-ui, logos-basecamp, logos-module-builder, lambda-prize, roadmap (content/testnets, content/blockchain), journeys, rfp.
- Logos web: logos.co, blog.logos.co, docs or forum sites, and @Logos_devs posts if reachable.
- Local clones under ../refs/ (`git log` and grep only; do not modify them).
Then go deeper only where the evidence changes the answer.
Use `gh` (read-only: `gh api`, `gh pr view`, `gh issue list`, `gh search`) and plain HTTP GETs. Do not post, comment, open issues or push anything.
</research_mode>

<citation_rules>
Back every answer with a direct link (URL with line anchor where possible) or `path:line` for local files, plus a short quote (at most 25 words).
Prefer primary sources (code, specs, maintainers' own words) over third-party posts.
Mark each answer VERIFIED (you read it), INFERRED (reasoned from evidence) or NOT FOUND.
Never present an inference as a fact. If two sources conflict, show both.
</citation_rules>

<structured_output_contract>
Write the report to docs/reviews/plan-0.4/codex-research.md and nothing else. Do not edit any other file.
Format:
1. A table: Question | Already answered? (yes / partly / no) | Answer in one or two sentences | Evidence (links) | Label.
2. "Questions we should not have asked": any of Q1–Q3 that were already answered publicly, with the source, so the team can follow up in Discord politely.
3. "New facts that change our plan": anything you found that contradicts or updates docs/dev/PLAN-0.4.md or docs/dev/status-2026-10-14.md, each with evidence.
4. "Searched but not found": where you looked for the NOT FOUND items.
Keep it compact. Highest-value findings first.
</structured_output_contract>

<default_follow_through_policy>
Do not stop to ask questions. If a source is unreachable, note it and continue with the next one.
</default_follow_through_policy>
```

---

## Prompt 2: adversarial review of the 0.4 plan

```xml
<task>
You are an independent senior reviewer (product, protocol and engineering). An AI agent wrote docs/dev/PLAN-0.4.md and docs/dev/status-2026-10-14.md for Logos Kit after Logos moved all λPrizes to testnet 0.4 (actor model, "end of November"). Be skeptical: find what is wrong, missing, over-built or risky. Do not confirm the agent's story; read the code and the sources.

The maintainer's intent, to hold the plan to:
"I'm going to build everything." One combined launch: a wallet with tokens, NFTs, private membership proofs and a Market. CLI parity. User experience matters most. When a network lacks a feature (tokens/NFTs on today's 0.3.1 testnet), the feature should degrade gracefully and switch on by itself when the network has it. A normal wallet links to the explorer after a transaction. No deadlines; do it right.

Review, in this order:
1. Facts. Re-check every claim in docs/dev/status-2026-10-14.md §2–§3 and PLAN-0.4.md §1 against primary sources. Examples: v0.3.1 removed token/ATA/AMM (PR #926); block header lost producer/signature (lez/common/src/block.rs at v0.3.1); PR #935's API (actor_entrypoint!, Response, SyncMessage/AsyncMessage, pending messages, recovery bindings, private accounts without identifiers); our 4 failing LEZ patches; explorer routes; Basecamp tags (0.3.0 = bbe5da0, 0.3.2 = f2fae63).
2. Strategy. Is "main on 0.3.1 with native LGO only + next/lez-0.4 on PR #935" the right call? Compare it with at least two alternatives: (a) skip 0.3.1 and move main to #935 now; (b) host our own token/ATA programs on 0.3.1. Judge by prize criteria, user experience, wasted work, and risk if #935 changes before 0.4.
3. Coverage. Map every LP-0021 and LP-0001 criterion (../refs/lambda-prize/prizes/LP-0021.md and LP-0001.md; refresh from GitHub first, since commit ad608bf changed them) to a stage in PLAN-0.4.md. List criteria with no stage or a weak exit proof.
4. Design. For the actor model, check the proposed designs: the gate (private NFT self-transfer with notify to a gate actor), the Market (the AMM pattern with escrow and pay-vault PDAs), vaults (one HD key node per private holding), Incoming → Claim for private receipts, recovery bindings, the testimonial port. Point out flaws, privacy leaks, or simpler options, with evidence from PR #935 (head 93e6cbf) code.
5. UX. Check the explorer/transaction-detail stage (X) and the per-network capability gating against what real wallets do (Phantom, Rainbow, Family, Glow). Is anything a top wallet does missing?
6. Order and dependencies. Is anything sequenced so that it blocks the prize path needlessly, or could start earlier?
</task>

<grounding_rules>
Ground every claim in files you read, commands you ran, or URLs you fetched. Cite path:line, commit SHA or URL.
Label each point VERIFIED, INFERRED or UNKNOWN. Do not present inferences as facts.
Read-only: do not modify code, do not git commit, push or open PRs, and do not post anywhere. You may run read-only commands (git log/show/diff, grep, gh api, cargo metadata). Do not run long builds.
</grounding_rules>

<dig_deeper_nudge>
After you find the first plausible issue, check for second-order failures: chain resets, stale wallet state, empty states, retries, proof timeouts, out-of-memory during proving, intent size and time limits in Basecamp, and what happens to existing installs.
</dig_deeper_nudge>

<structured_output_contract>
Write the report to docs/reviews/plan-0.4/codex-review.md and nothing else. Do not edit any other file.
Format:
1. Verdict in three sentences.
2. Top 10 findings ranked by impact. Each: what is wrong or missing, evidence, the concrete change to the plan, label.
3. Fact check table: claim | correct? | evidence.
4. Criteria coverage table: criterion | stage | gap.
5. Alternatives considered (strategy) with your recommendation and why.
Compact. Highest impact first.
</structured_output_contract>

<default_follow_through_policy>
Do not stop to ask questions. If something can't be checked, say so and move on.
</default_follow_through_policy>
```
