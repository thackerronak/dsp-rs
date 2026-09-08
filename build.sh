#!/bin/sh
set -e

# The Dockerfile is multi-stage: the connector is compiled inside the image, so no musl
# target or musl linker is needed on the host. No --platform either — the image is built
# for the host architecture.
docker build -t connector:latest .
