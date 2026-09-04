# Why DSP? (and why a central Issuer isn't "centralized")

_Part of the [DSP guide](README.md) · Background_

## "They can just connect directly" — true, for exactly two parties

For **two** parties, a direct API is simpler. But real data ecosystems have
**hundreds** of parties, each both buyer and seller. Direct integration means an
N×N explosion (~5,000 bespoke pipes for 100 companies), each with its own auth,
its own contract format (a PDF no software can enforce), and its own data format.

DSP replaces those pipes with **one shared language**: build to the spec once,
trade with anyone else who did the same — no prior relationship required. Like
**shipping containers** — standardize once, and any container fits any ship,
crane, and truck worldwide.

## The 5 things DSP gives you that a direct connection does not

1. **Interoperability without prior agreement** — join the network, not each peer.
2. **Data sovereignty** — the seller keeps control *after* handoff; machine-readable
   policies travel with the deal and are enforced by the connector.
3. **Software-executable contracts** — negotiation yields a signed digital
   agreement; no agreement → no data.
4. **Decentralized trust** — no central broker holds your data; each party proves
   its own identity cryptographically.
5. **Auditability & symmetry** — every step is an explicit, logged, standardized
   message; the same connector buys and sells.

## "But Catena-X has a central Issuer — isn't that centralized?"

Two different layers:

- **Identity issuance** (who is a legitimate member?) → needs a **central trust
  anchor** (an Issuer). Minimal, one-time, at onboarding.
- **Data exchange** (how does data flow?) → **fully decentralized, peer-to-peer.**

Analogy: your **passport** is issued centrally (a government), but you then
**travel freely** — the passport office isn't the airline, hotel, or customs
officer for every trip, and never sees your journeys. Same as HTTPS: a small set
of trusted **Certificate Authorities** vouch for domains, but your traffic never
routes through them.

In this codebase the trust anchor is a **list** — `allowed_issuers`
(`src/auth/mod.rs`) — so a dataspace can trust multiple issuers. Everything about
the *data* (catalog, negotiation, transfer, enforcement) is decentralized; the
Issuer only vouches for identity and never sees the data.

### What's central vs. decentralized

| Aspect | Central? | Why |
|--------|----------|-----|
| Issuing membership/BPN credentials | ✅ | One agreed root of trust so a stranger's credential means something |
| Governance / rulebook | ✅ | Everyone must agree on the standards |
| Your data, the catalog, negotiation, transfer, policy enforcement | ❌ | Stays on your connector; flows directly to the counterparty |

> **Catena-X mapping.** The trusted Issuer is the central **Catena-X Issuer
> service**, operated by the network's Operating Company (e.g. Cofinity-X) under
> the governance of the *Catena-X Automotive Network e.V.* It issues **Membership**,
> **BPN** (Business Partner Number), and use-case **Framework** credentials. **DCP**
> = how you *get* your Catena-X ID; **DSP** (this repo) = how you *use* it to trade
> data.

---

**Next:** [Stage 1 — Catalog →](catalog.md) · [Index](README.md)
