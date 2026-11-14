//! BOM stores, Apple's block-indexed container: the asset catalogs `actool`
//! writes and the bills of materials in installer packages. A store holds
//! numbered blocks and variables that name some of them; what the blocks
//! hold is up to its writer.

/// A BOM store being put together: blocks, and variables that name them.
#[derive(Default)]
pub(crate) struct Bom {
    /// Every block but the null block 0.
    blocks: Vec<Vec<u8>>,
    variables: Vec<(&'static str, u32)>,
}

impl Bom {
    /// Add a block, returning its number.
    pub(crate) fn add(&mut self, data: Vec<u8>) -> u32 {
        self.blocks.push(data);
        count(self.blocks.len())
    }

    /// Fill in block `index`, added before what it holds was known.
    pub(crate) fn set(&mut self, index: u32, data: Vec<u8>) {
        let slot = index
            .checked_sub(1)
            .and_then(|i| self.blocks.get_mut(usize::try_from(i).ok()?));
        if let Some(block) = slot {
            *block = data;
        }
    }

    /// The number the next block added gets.
    pub(crate) fn next(&self) -> u32 {
        count(self.blocks.len()) + 1
    }

    pub(crate) fn variable(&mut self, name: &'static str, index: u32) {
        self.variables.push((name, index));
    }

    /// The store: a 512-byte header area, the blocks 16-byte aligned, the
    /// variables, then the block index with two empty free-list entries.
    pub(crate) fn finish(self) -> Vec<u8> {
        let align = |out: &mut Vec<u8>| out.resize(out.len().next_multiple_of(16), 0);
        let mut out = vec![0u8; 512];
        let mut addresses = vec![(0u32, 0u32)];
        for block in &self.blocks {
            align(&mut out);
            addresses.push((count(out.len()), count(block.len())));
            out.extend_from_slice(block);
        }
        align(&mut out);
        let variables_offset = count(out.len());
        let mut variables = count(self.variables.len()).to_be_bytes().to_vec();
        for (name, index) in &self.variables {
            variables.extend_from_slice(&index.to_be_bytes());
            variables.push(u8::try_from(name.len()).unwrap_or(u8::MAX));
            variables.extend_from_slice(name.as_bytes());
        }
        out.extend_from_slice(&variables);
        align(&mut out);
        let index_offset = count(out.len());
        let mut index = count(addresses.len()).to_be_bytes().to_vec();
        for (address, length) in &addresses {
            index.extend_from_slice(&address.to_be_bytes());
            index.extend_from_slice(&length.to_be_bytes());
        }
        index.extend_from_slice(&2u32.to_be_bytes());
        index.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&index);
        let mut header = b"BOMStore".to_vec();
        for value in [
            1,
            count(self.blocks.len()),
            index_offset,
            count(index.len()),
            variables_offset,
            count(variables.len()),
        ] {
            header.extend_from_slice(&value.to_be_bytes());
        }
        out[..header.len()].copy_from_slice(&header);
        out
    }
}

/// A size or count as a store records it, in 32 bits.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
