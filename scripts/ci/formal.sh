#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Shared CI entry point; never execute an unchecked model checker.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cache="$root/.cache/tla"
jar="$cache/tla2tools.jar"
url="https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar"
sha256="936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
mkdir -p "$cache" "$root/target/assurance/formal"
if [ ! -f "$jar" ]; then
  curl -sSfL --retry 3 -o "$jar.part" "$url"
  mv "$jar.part" "$jar"
fi
printf '%s  %s\n' "$sha256" "$jar" | sha256sum --check -
java -version 2>&1 | tee "$root/target/assurance/formal/java.log"
# No -deadlock: that option DISABLES deadlock checking.
java -XX:+UseParallelGC -jar "$jar" -workers auto -metadir "$cache/states" \
  "$root/formal/Cache.tla" 2>&1 | tee "$root/target/assurance/formal/tlc.log"
