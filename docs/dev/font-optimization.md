# Font Optimization Documentation

This document explains how SweetPad optimizes its icon font to reduce the extension size by 99.3%.

## Overview

SweetPad uses a custom font subsetting process to include only the icons actually used by the extension, reducing the font file from ~1.2MB to ~7.6KB.

## Workflow

### 1. Font Analysis

The optimization process analyzes `package.json` to identify all icon references:

- Scans the `contributes.icons` section
- Extracts Unicode codepoints for each icon
- Builds a list of required characters

### 2. Font Subsetting

Using Python fonttools via `uv`, the process:

- Creates a subset containing only used characters
- Preserves the original font as backup
- Generates an optimized `.woff` file

### 3. Verification

The verification step ensures:

- All required font files exist
- Package.json references the correct font paths
- The subset font contains all expected icons
- Size reduction is achieved as expected

## Commands

```bash
# Optimize the font (creates subset)
npm run optimize-font

# Verify the optimization
npm run verify-font

# Publish with verification
npm run publish-patch
```

## Font Files

| File | Purpose | Size |
|------|---------|------|
| `images/icons/tabler-icons.original.woff` | Original font (backup) | ~1.2MB |
| `images/icons/tabler-icons.woff` | Optimized subset | ~7.6KB |

## Implementation Details

### Dependencies

- **uv**: Python package manager for fonttools
- **fonttools**: Python library for font manipulation
- **pyftsubset**: Command-line tool for font subsetting

### Process Flow

1. **Extract Icons**: Parse package.json for icon definitions
2. **Create Subset**: Run pyftsubset with Unicode ranges
3. **Verify Configuration**: Check package.json font paths
4. **Validate Content**: Ensure all icons are present in subset

### Integration

Font verification is automatically run during:

- Manual verification: `npm run verify-font`
- Pre-publish checks: `npm run publish-patch`

This ensures the extension is always published with correctly optimized fonts.
