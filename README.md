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

Then run the TCK (Docker image, or Gradle `:dcp-tck:shadowJar` then `java -jar dcp-tck-runtime.jar -config
tck/dcp-rs.tck.properties`, per its own README) against the running connector.

This has been run end to end (Gradle build, JDK 17): **85 of 117 test cases pass.** That run found and fixed six real
conformance bugs, now fixed on this branch — see [docs/limitations.md](docs/limitations.md), "DCP TCK conformance",
for the full list of what was fixed and what's left. What's left splits into a genuine, security-relevant gap in how
a presented credential's content is checked (expiry, revocation, subject binding), an already-known gap (`kid` is
never used to pick a verification key), an inherent conflict between this connector's single-identity architecture
and the TCK's three-separate-parties harness model, and a handful of real unimplemented CredentialService/
IssuerService business rules. None of it is a config problem in the committed `tck/dcp-rs.tck.properties` /
`tck/config.json`.
