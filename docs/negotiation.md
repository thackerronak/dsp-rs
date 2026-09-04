# Stage 3 — Negotiation (sign the contract) 🤝📝

_Part of the [DSP guide](README.md) · Stage 3 of 4_

Like signing a B2B contract **by registered mail**: each side mails a signed
letter, every letter advances a shared state machine, and each carries **case
numbers** (PIDs) so both sides match the correspondence. No Issuer involved — pure
peer-to-peer. Defined in `src/negotiation/`.

## PIDs (Process IDs)

**PID = Process ID** — the reference number for one negotiation. There are two,
because each side keeps its own:
- `consumerPid` — created by the consumer at the start (its purchase-order number).
- `providerPid` — minted by the provider when it first sees the request (its
  sales-order number).

Every message carries **both**. (The initial request has no `providerPid` yet — the
provider mints it and returns it in the `201`.)

## State machine

```
REQUESTED ──► (OFFERED ──► ACCEPTED) ──► AGREED ──► VERIFIED ──► FINALIZED ✅
                                    └────────► TERMINATED ❌ (either side, any time)
```

## Endpoints — the endpoint lives on the _receiver_

(`src/negotiation/mod.rs`)

| Endpoint | Hosted by | Message received | Result |
|----------|-----------|------------------|--------|
| `POST /negotiations/request` | Provider | initial `ContractRequestMessage` | `201`, REQUESTED (mints `providerPid`) |
| `POST /negotiations/{providerPid}/request` | Provider | counter `ContractRequestMessage` | ACCEPTED |
| `POST /negotiations/{consumerPid}/offers` | Consumer | `ContractOfferMessage` | OFFERED |
| `POST /negotiations/{consumerPid}/agreement` | Consumer | `ContractAgreementMessage` | AGREED |
| `POST /negotiations/{providerPid}/agreement/verification` | Provider | `ContractAgreementVerificationMessage` | VERIFIED |
| `POST /negotiations/{consumerPid}/events` | Consumer | `ContractNegotiationEventMessage` (FINALIZED) | FINALIZED |
| `POST /negotiations/{pid}/termination` | Either | `ContractNegotiationTerminationMessage` | TERMINATED |
| `GET /negotiations/{pid}` | Either | (read current state) | — |

## Happy path

1. **Request** (`contract_request`, `src/negotiation/provider.rs`) — InsureCo POSTs a
   `ContractRequestMessage` with the `offer` (policy `@id` + dataset `target`) and a
   `callbackAddress`. Provider mints `providerPid`, returns `201` REQUESTED.
2. **Policy engine** (`src/negotiation/provider.rs`, `negotiate_policy`) — the
   provider compares the consumer's *requested* terms against the dataset's
   *required* policy. Compatible → advance; incompatible → auto-**TERMINATED**
   (`"policy negotiation failed"`). **This is data sovereignty enforced in code.**
3. **Agreement** — provider callbacks to the consumer's `/agreement` with a signed
   `Agreement` that has its own `@id` (needed in [Stage 4](transfer.md)),
   `assigner`, `assignee`, `target`, and `permission`/`constraint`.
4. **Verification** — consumer countersigns via `/agreement/verification` → VERIFIED.
5. **Finalized** — provider callbacks to the consumer's `/events` with
   `eventType: FINALIZED` → the `Agreement` is now binding on both connectors.

## Why it's asynchronous

Handlers don't do the work inline — they queue the message and immediately ACK
(`200`/`201`). A **background loop** ticking every second (`handle_negotiations` →
`process_negotiations`, `src/negotiation/mod.rs`) finds pending negotiations and
`.tick()`s them, sending the next letter to the other side's `callbackAddress`.
This distributed state machine scales to many concurrent deals without holding
connections open — hence every message needs a `callbackAddress` ("mail replies
here").

## Why verify + finalize (the double handshake)

Provider agrees → consumer verifies → provider finalizes. Both parties provably,
cryptographically consented (every letter is signed via the
[Stage 2](auth-and-identity.md) badge) — a **non-repudiable** contract you can't
later disown.

## Termination

Either side can send a `ContractNegotiationTerminationMessage` to
`/negotiations/{pid}/termination` at any point (e.g. policy mismatch, changed mind,
error) → TERMINATED. This is the clean "call off the deal" path.

> 🚗 **Catena-X:** the negotiated `offer` is a Catena-X **usage/access policy** (e.g.
> `FrameworkAgreement.traceability`, `Membership`, a BPN constraint). The resulting
> `Agreement` `@id` is the **contract agreement** stored in the EDC, quoted later to
> start a transfer.

---

**Prev:** [← Stage 2 — Auth & Identity](auth-and-identity.md) · **Next:** [Stage 4 — Transfer →](transfer.md) · [Index](README.md)
