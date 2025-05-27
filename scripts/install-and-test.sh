#!/bin/bash

echo "🔄 Installing SweetPad extension..."
code --install-extension sweetpad-0.1.66.vsix

echo "📂 Opening SPM test project..."
code tests/examples/sweetpad-spm

echo "✅ Done! Extension installed and SPM project opened."
echo ""
echo "💡 You can now test the SPM functionality in the opened project." 