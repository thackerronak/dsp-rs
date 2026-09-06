#!/bin/sh

# build the self-contained connector image (compiles inside the builder stage)
docker build --platform linux/amd64 -t connector:latest -f Dockerfile .
