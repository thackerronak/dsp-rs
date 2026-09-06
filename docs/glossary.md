# Glossary

_Part of the [DSP guide](README.md) · Reference_

| Term | Meaning |
|------|---------|
| **DSP** | *Dataspace Protocol* — the standard this connector implements (version 2025-1) for catalog, contract negotiation, and transfer between connectors. |
| **DCP** | *Decentralized Claims Protocol* — the identity/credential protocol used to *obtain* credentials and present them (the `/auth/*` handshake). DSP uses the resulting tokens. |
| **Connector** | A participant's automated "trade desk" that speaks DSP. Every connector is both producer and consumer. |
| **Producer / Provider** | The party offering data (the seller). |
| **Consumer** | The party requesting data (the buyer). |
| **DID** | *Decentralized Identifier*, e.g. `did:web:party-a-connector%3A3000`. A self-hosted, resolvable identity. |
| **`did:web`** | A DID method where the identity document is hosted at the party's own domain (`/.well-known/did.json`). |
| **DID document (`did.json`)** | The public "passport page" listing a party's public key(s), used to verify its signatures. |
| **VC** | *Verifiable Credential* — a signed claim about a party (e.g. identity, membership, BPN), issued by a trusted Issuer and held in the connector's credential store. |
| **VP** | *Verifiable Presentation* — a package of one or more VCs a party presents to prove something, without revealing more than needed. |
| **Issuer** | The trusted authority that signs VCs (the trust anchor). Each native connector can issue; in Catena-X, the central Issuer operated by the network's Operating Company. |
| **Verifier** | The component that checks a presented VP is genuine (the connector's native verifier, behind `POST /auth/token`). |
| **DCP** | *Decentralized Claims Protocol* — the native, synchronous **pull** protocol the connector uses: a Self-Issued ID Token authorizes the verifier to `POST` a `PresentationQueryMessage` to the holder's Credential Service, which returns a VP inline. |
| **Wallet** | The connector's built-in native DCP wallet — issuer + holder + verifier + STS behind one DID. |
| **SI Token** | *Self-Issued ID Token* — a short-lived ES256 JWT (`iss == sub`) a party mints to authenticate to a peer's `/auth/token` or Credential Service. |
| **ES256** | The ECDSA-with-P-256 signature algorithm used to sign the access/transfer JWTs. |
| **ODRL** | *Open Digital Rights Language* — how usage **policies** are expressed (permissions, constraints like "spatial isPartOf EU"). |
| **DCAT** | *Data Catalog Vocabulary* — how the **catalog** is expressed (`dcat:Catalog`, `dcat:dataset`, `dcat:distribution`). |
| **Offer / Agreement** | An **Offer** is proposed usage terms (in the catalog/negotiation). An **Agreement** is the signed, mutually-accepted contract with a unique `@id`. |
| **PID** | *Process ID* — the reference number for one negotiation/transfer. Two exist: `consumerPid` and `providerPid`. |
| **Callback address** | The base URL a party tells the other to send follow-up (async) messages to. |
| **Control plane / Data plane** | Control plane = the `/transfers/*` coordination messages; data plane = the `/pull` proxy that moves the actual bytes. |
| **DataAddress** | The endpoint + token a consumer receives to fetch (PULL) the data. Catena-X calls this an **EDR**. |
| **EDR** | *Endpoint Data Reference* — Catena-X's name for the DataAddress (endpoint + short-lived token). |
| **EDC** | *Eclipse Dataspace Connector* — the reference DSP connector; `dsp-rs` is a Rust sibling. **Tractus-X EDC** is the Catena-X flavor. |
| **BPN / BPNL** | *Business Partner Number (Legal entity)* — the unique company identifier in Catena-X, carried in a BPN credential. |
| **Federation** | This repo's static config of which remote connectors to sync catalogs from. |
| **TCK** | *Technology Compatibility Kit* — the official DSP conformance test suite; this repo builds a `tck` feature to run against it. |

---

**Prev:** [← Stage 4 — Transfer](transfer.md) · **Next:** [Limitations →](limitations.md) · [Index](README.md)
