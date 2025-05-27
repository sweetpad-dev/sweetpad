#!/bin/bash

set -e

# run build
npm run build

# Extract version from package.json
VERSION=$(node -p "require('./package.json').version")
echo "📋 Using version: $VERSION"

echo "🍭 Creating VSIX package manually..."

# Clean up any existing VSIX files
rm -f *.vsix

# Create temporary directory for VSIX structure
TEMP_DIR=$(mktemp -d)
VSIX_DIR="$TEMP_DIR/vsix"
mkdir -p "$VSIX_DIR"

echo "📦 Preparing VSIX structure..."

# Create extension directory structure
mkdir -p "$VSIX_DIR/extension"

# Copy extension files to the extension subdirectory
cp -r out "$VSIX_DIR/extension/"
cp package.json "$VSIX_DIR/extension/"
cp README.md "$VSIX_DIR/extension/"
cp LICENSE.md "$VSIX_DIR/extension/"
cp CHANGELOG.md "$VSIX_DIR/extension/"
cp -r images "$VSIX_DIR/extension/"

# Create [Content_Types].xml
cat > "$VSIX_DIR/[Content_Types].xml" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json" ContentType="application/json" />
  <Default Extension="js" ContentType="application/javascript" />
  <Default Extension="png" ContentType="image/png" />
  <Default Extension="md" ContentType="text/markdown" />
  <Default Extension="txt" ContentType="text/plain" />
  <Default Extension="vsixmanifest" ContentType="text/xml" />
</Types>
EOF

# Create extension.vsixmanifest with dynamic version
cat > "$VSIX_DIR/extension.vsixmanifest" << EOF
<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="sweetpad" Version="$VERSION" Publisher="sweetpad" />
    <DisplayName>SweetPad (iOS/Swift development)</DisplayName>
    <Description xml:space="preserve">Develop Swift/iOS projects in VS Code</Description>
    <Tags>swift,ios,xcode,development,mobile</Tags>
    <Categories>Formatters,Linters,Extension Packs,Programming Languages,Other</Categories>
    <GalleryFlags>Preview</GalleryFlags>
    <License>extension/LICENSE.md</License>
    <Icon>extension/images/logo.png</Icon>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" Version="[1.85.0,)" />
  </Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/images/logo.png" Addressable="true" />
  </Assets>
</PackageManifest>
EOF

echo "📦 Creating ZIP archive..."

# Create the VSIX file (which is just a ZIP with .vsix extension)
cd "$VSIX_DIR"
zip -r "../sweetpad-$VERSION.vsix" . > /dev/null
cd - > /dev/null

# Move the VSIX file to the project root
mv "$TEMP_DIR/sweetpad-$VERSION.vsix" ./

# Clean up
rm -rf "$TEMP_DIR"

echo "✅ VSIX package created: sweetpad-$VERSION.vsix"

# Verify the file exists and show its size
if [ -f "sweetpad-$VERSION.vsix" ]; then
    SIZE=$(ls -lh "sweetpad-$VERSION.vsix" | awk '{print $5}')
    echo "📊 Package size: $SIZE"
    echo ""
    echo "🚀 Installation commands:"
    echo "   code --install-extension sweetpad-$VERSION.vsix"
    echo "   # or"
    echo "   cursor --install-extension sweetpad-$VERSION.vsix"
else
    echo "❌ Failed to create VSIX package"
    exit 1
fi 


echo "🔄 Installing SweetPad extension..."
code --install-extension "sweetpad-$VERSION.vsix"


echo "✅ Done! Extension installed"
echo ""
echo "💡 You can now reload window to see the extension in action." 