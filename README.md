# DSP Implementation in Rust

## Building

Use the provided [build.sh](build.sh) script for compiling the connector binary and creating a local Docker container.

## Setup and usage instructions

The `docker-compose` folder contains a demo setup with two dataspace participants:

- [Setting up SSI wallets and connector](docker-compose/SETUP.md)
- [Negotiating a contract and accessing a dataset](docker-compose/USAGE.md)

## Run connector in TCK mode

```bash
RUST_LOG=debug CONFIG_PATH="tck/config.json" cargo run --bin dsp-rs --features tck
```

The TCK configuration is provided in [tck/dsp-rs.tck.properties](tck/dsp-rs.tck.properties). Further information about
running the test suite can be found on the official [repo](https://github.com/eclipse-dataspacetck/dsp-tck).

## Run connector against DCP TCK

The DCP TCK ([eclipse-dataspacetck/dcp-tck](https://github.com/eclipse-dataspacetck/dcp-tck)) is the DCP sibling of
the suite above: it drives this connector's CredentialService, IssuerService and verifier as a black box, rather than
checking messages against a schema. Unlike DSP TCK mode, run the plain build — `--features tck` stubs out
`AuthClaims` for the DSP-only catalog/negotiation/transfer endpoints, which this TCK never calls, and there is no
reason to carry that stub into a run whose entire point is exercising the real DCP auth path:

```bash
RUST_LOG=debug CONFIG_PATH="tck/config.json" cargo run --bin dsp-rs
```

Then run the TCK (Docker image or Gradle `shadowJar`, per its own README) with the properties in
[tck/dcp-rs.tck.properties](tck/dcp-rs.tck.properties). Two things there need a live run to fill in — the TCK mints
its own DIDs for the mock services it stands up, and prints them at startup rather than exposing them as static
config:

- `dataspacetck.did.thirdparty`, marked `CHANGE_ME` in the properties file.
- `tck/config.json`'s `allowed_issuers`, currently set to `did:web:localhost%3A8080` (derived the same way this
  connector derives its own DID — from `dataspacetck.callback.address`, `http://localhost:8080`). That is this
  connector's best guess at the DID the TCK's own mock issuer signs credentials with in the Verifier test package;
  confirm it against the TCK's log and adjust if the "authorized" verifier cases come back rejected.

This integration has not been run end to end in this environment (no Docker daemon available here) — treat it as a
first pass to validate against a real run, not a confirmed-working setup. One known, unavoidable gap either way: this
connector issues no revocation status list, so the TCK's revocation test cases will fail regardless of configuration
(see [docs/limitations.md](docs/limitations.md)).
