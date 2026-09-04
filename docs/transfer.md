# Stage 4 — Transfer (pick up the goods) 📦🔑

_Part of the [DSP guide](README.md) · Stage 4 of 4_

You've signed the contract; now collect the data. Key new idea: **control plane
vs. data plane.**

| Plane | Carries | In this repo |
|-------|---------|--------------|
| **Control plane** | coordination messages ("start", "complete", tokens, state) | `/transfers/*` |
| **Data plane** | the actual bytes of the dataset | the `/pull/*` reverse proxy |

DSP separates them: the `/transfers/*` handshake never carries data — it
*negotiates access* to it; the bytes flow through a different door (`/pull`).

Two styles exist:
- **PULL** (`HttpData-PULL`, the demo path) — provider hands you an endpoint +
  token, you fetch. Data plane is the **provider's** `/pull`.
- **PUSH** — the provider sends data to **your** endpoint. The consumer supplies a
  `push` endpoint + token in the start message (`src/transfer/consumer.rs`); the
  data-plane direction flips. (PULL is the fully-wired path in this repo — see
  [Limitations](limitations.md).)

## State machine

```
REQUESTED ──► STARTED ⇄ SUSPENDED ──► COMPLETED
                  └──────────────────► TERMINATED (either side, any time)
```

## Endpoints — the receiver hosts each

(`src/transfer/mod.rs`)

| Endpoint | Hosted by | Message | Result |
|----------|-----------|---------|--------|
| `POST /transfers/request` | Provider | `TransferRequestMessage` (quotes the Agreement ID) | `201`, REQUESTED |
| `POST /transfers/{consumerPid}/start` | Consumer | `TransferStartMessage` (carries `DataAddress`: endpoint + token) | STARTED |
| `POST /transfers/{consumerPid}/suspension` | Consumer | `TransferSuspensionMessage` | SUSPENDED |
| `POST /transfers/{providerPid}/completion` | Provider | `TransferCompletionMessage` | COMPLETED |
| `POST /transfers/{pid}/termination` | Either | `TransferTerminationMessage` | TERMINATED |
| `GET /transfers/{providerPid}` | Either | (read state) | — |
| `ANY /pull/*` | Provider (data plane) | the actual data request | streams bytes |

## Happy path (PULL)

1. **Request** — InsureCo POSTs a `TransferRequestMessage` quoting `agreementId`
   (from [Stage 3](negotiation.md)) and `format: "HttpData-PULL"`. **No finalized
   agreement → no transfer.** Provider returns `201` REQUESTED.
2. **Start** (`src/transfer/provider.rs`) — provider mints a **transfer token**
   scoped to this transfer (`new_transfer_token`, with `sub` = the transfer's
   `providerPid`) and callbacks to the consumer's `/start` with a `DataAddress`: the
   `/pull` endpoint + the token. State → STARTED.
3. **Pull (data plane)** — InsureCo calls `GET /pull/...` with
   `Authorization: <transfer_token>`. The reverse proxy (`src/reverse_proxy/mod.rs`,
   `find_data_asset_target`):
   - decodes the token → `sub` = `providerPid`;
   - looks up the transfer — it **must be STARTED**, else `401`;
   - resolves the `Agreement` → the **real backend dataset URL** (hidden from the
     consumer);
   - **streams** the data back (HTTP and WebSocket supported). Repeat while STARTED.
4. **Completion** — InsureCo POSTs `TransferCompletionMessage` to the provider's
   `/completion` → COMPLETED.

## Suspension & termination (revoking access)

The provider (or consumer) can send **suspension** (`.../{pid}/suspension`) to pause
or **termination** (`.../{pid}/termination`) to stop. Because every pull re-checks
the transfer is `STARTED`, suspension/completion/termination **locks the token
instantly** — the next `/pull` returns `401`. This is live, revocable access.

## Why this design = sovereignty, made real

- The consumer **never sees the real data URL** — only `/pull`; the proxy holds the
  true address.
- Access is **live and revocable** (see above). Contrast a plain download: once the
  file is out, it's gone.
- The token is **scoped to one contract** (`sub → providerPid → agreement →
  dataset`); it can't reach any other dataset.
- **The agreement gates everything** — the [Stage 3](negotiation.md) contract *is*
  the access key.

> 🚗 **Catena-X:** the `DataAddress` (endpoint + token) is Catena-X's **EDR**
> (Endpoint Data Reference); `/pull` is the EDC **data plane**, `/transfers/*` the
> **control plane** — same token-gated pull.

---

**Prev:** [← Stage 3 — Negotiation](negotiation.md) · **Next:** [Glossary →](glossary.md) · [Index](README.md)
