use crate::world::BLOCK_AIR;
use flywheelmc_common::prelude::*;
use protocol::value::{ Var32, BlockState };
use protocol::value::{ ChunkSection as PtcChunkSection, PalettedContainer, PaletteFormat };
use protocol::registry::RegEntry;


#[derive(Clone)]
pub struct ChunkSection {
    data  : Vec<u8>,
    dirty : BTreeSet<u16>
}

impl ChunkSection {

    pub fn empty() -> Self {
        let mut data = Vec::new();
        section_write_data_run(&mut data, 4096, BLOCK_AIR);
        Self {
            data,
            dirty : BTreeSet::new()
        }
    }

    #[inline]
    pub fn iter<'l>(&'l self) -> SectionIter<'l> {
        SectionIter {
            iter          : self.data.iter().cloned(),
            current_run   : 0,
            current_block : BLOCK_AIR
        }
    }

    pub fn checked_get(&self, linear_xyz : u16) -> Option<RegEntry<BlockState>> {
        self.iter().nth(linear_xyz as usize)
    }

    #[inline]
    pub fn get(&self, linear_xyz : u16) -> RegEntry<BlockState> {
        self.checked_get(linear_xyz).expect("called ChunkSection::get with out-of-range block index")
    }

    pub fn checked_get_xyz(&self, dx : u8, dy : u8, dz : u8) -> Option<RegEntry<BlockState>> {
        self.checked_get(in_section_block_linearise(dx, dy, dz))
    }

    pub fn get_xyz(&self, dx : u8, dy : u8, dz : u8) -> RegEntry<BlockState> {
        self.get(in_section_block_linearise(dx, dy, dz))
    }

    pub fn writer<'l>(&'l mut self) -> SectionWriter<'l> {
        SectionWriter {
            section : self,
            blocks  : BTreeMap::new()
        }
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub(crate) fn ptc_chunk_section(&self) -> PtcChunkSection {
        let mut block_count = 0;
        let run_len = u16::from_ne_bytes([self.data[0], self.data[1]]);
        let block_states = if (run_len == 4096) {
            let (entry, _,) = Var32::decode_iter(&mut self.data[2..].iter().cloned()).unwrap();
            if (entry.as_i32() != 0) { block_count = 4096; }
            PalettedContainer {
                bits_per_entry : 0,
                format         : PaletteFormat::SingleValued { entry : unsafe { RegEntry::new_unchecked(entry.as_i32() as u32) } }
            }
        } else {
            let mut data   = [unsafe { RegEntry::new_unchecked(0) }; 4096];
            let mut max_id = 0;
            for (i, block) in self.iter().enumerate() {
                let id = block.id();
                if (id != 0) { block_count += 1; }
                max_id = max_id.max(id);
                data[i] = block;
            }
            PalettedContainer {
                bits_per_entry : 15,
                format         : PaletteFormat::Direct { data }
            }
        };
        PtcChunkSection {
            block_count,
            block_states,
            biomes       : PalettedContainer {
                bits_per_entry : 0,
                format         : PaletteFormat::SingleValued { entry : unsafe { RegEntry::new_unchecked(0) } }
            }
        }
    }

}


pub struct SectionIter<'l> {
    iter          : iter::Cloned<slice::Iter<'l, u8>>,
    current_run   : u16,
    current_block : RegEntry<BlockState>
}

impl<'l> Iterator for SectionIter<'l> {

    type Item = RegEntry<BlockState>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if (self.current_run > 0) {
                self.current_run -= 1;
                return Some(self.current_block);
            } else {
                self.current_run = u16::from_ne_bytes([self.iter.next()?, self.iter.next()?]);
                let (entry, _,) = Var32::decode_iter(&mut self.iter).ok()?;
                self.current_block = unsafe { RegEntry::new_unchecked(entry.as_i32().cast_unsigned()) };
            }
        }
    }

}


pub struct SectionWriter<'l> {
    section : &'l mut ChunkSection,
    blocks  : BTreeMap<u16, RegEntry<BlockState>>
}

impl<'l> SectionWriter<'l> {

    #[inline]
    pub fn set(&mut self, linear_xyz : u16, block : RegEntry<BlockState>) {
        self.blocks.insert(linear_xyz, block);
    }

    pub fn set_xyz(&mut self, dx : u8, dy : u8, dz : u8, block : RegEntry<BlockState>) {
        self.set(in_section_block_linearise(dx, dy, dz), block)
    }

}

impl<'l> Drop for SectionWriter<'l> {

    fn drop(&mut self) {
        if (! self.blocks.is_empty()) {
            let mut new_data  = Vec::new();
            let mut run_len   = 0;
            let mut run_block = None;
            for block in self.section.iter().enumerate().map(|(i, block,)|
                self.blocks.get(&(i as u16))
                    .map_or(block, |block| *block)
            ) {
                if (run_block.is_some_and(|run_block| run_block == block)) {
                    run_len += 1;
                } else {
                    if let Some(run_block) = run_block {
                        section_write_data_run(&mut new_data, run_len, run_block);
                    }
                    run_len   = 1;
                    run_block = Some(block);
                }
            }
            if let Some(run_block) = run_block {
                section_write_data_run(&mut new_data, run_len, run_block);
            }
            self.section.data = new_data;
            self.section.dirty.extend(self.blocks.keys());
        }
    }

}


fn in_section_block_linearise(dx : u8, dy : u8, dz : u8) -> u16 {
    (((dy & 0b00001111) as u16) << 8)
    | (((dz & 0b00001111) as u16) << 4)
    | ((dx & 0b00001111) as u16)
}

fn in_section_block_delinearise(l : u16) -> [u8; 3] {
    [
        (l as u8) & 0b00001111,
        ((l >> 8) as u8) & 0b00001111,
        ((l >> 4) as u8) & 0b00001111
    ]
}

fn section_write_data_run(data : &mut Vec<u8>, len : u16, block : RegEntry<BlockState>) {
    if (len > 0) {
        data.extend(len.to_ne_bytes());
        Var32::new(block.id() as i32).extend_bytes(data);
    }
}
