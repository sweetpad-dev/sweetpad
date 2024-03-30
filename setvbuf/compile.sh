#!/bin/bash
set -Eeuo pipefail

# Ensure the output directory exists
mkdir -p out

# Path to your source file
SOURCE_FILE="setvbuf/setvbuf.c"

# Set locations
TEMP_DIR="./.temp"
OUT_DIR="./out"

# Ensure the temporary directory exists
mkdir -p $TEMP_DIR

# Compile for arm64 architecture
clang -O2 -fpic -shared -arch arm64 -o $TEMP_DIR/setvbuf_arm64.so $SOURCE_FILE

# Compile for x86_64 architecture
clang -O2 -fpic -shared -arch x86_64 -o $TEMP_DIR/setvbuf_x86_64.so $SOURCE_FILE

# Create a Universal Binary from the architecture-specific files
lipo -create -output $OUT_DIR/setvbuf_universal.so $TEMP_DIR/setvbuf_arm64.so $TEMP_DIR/setvbuf_x86_64.so

# Optionally, remove the architecture-specific shared libraries if not needed
rm $TEMP_DIR/setvbuf_arm64.so $TEMP_DIR/setvbuf_x86_64.so

# Remove the temporary directory if not needed
rm -r $TEMP_DIR

echo "Universal binary created at out/setvbuf_universal.so"

