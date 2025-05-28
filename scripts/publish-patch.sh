#!/bin/bash

# Exit on error
set -e

# Step 1: Bump the version
echo "Bumping version..."
VERSION=$(npm version patch)

# Step 2: Push changes to main
echo "Pushing to main..."
git push origin main

# Step 4: Publish tag
echo "Publishing tag $VERSION..."
git push origin $VERSION

# Step 5: Open GitHub Actions page
echo "Opening GitHub Actions page..."
echo "-> https://github.com/KayodeOgundimu-DoorDashSWE/sweetpad/actions"
