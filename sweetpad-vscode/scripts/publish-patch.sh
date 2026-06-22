#!/bin/bash

# Exit on error
set -e

# Step 1: Verify font optimization is correct
echo "Verifying font optimization..."
npm run verify-font

# Step 2: Bump the version
echo "Bumping version..."
VERSION=$(npm version patch)

# Step 3: Push changes to main
echo "Pushing to main..."
git push origin main

# Step 4: Publish tag
echo "Publishing tag $VERSION..."
git push origin $VERSION

# Step 5: Open GitHub Actions page
echo "Opening GitHub Actions page..."
echo "-> https://github.com/sweetpad-dev/sweetpad/actions"
