use crate::world::{ BLOCK_AIR, in_section_block_linearise, in_section_block_delinearise };
use flywheelmc_common::prelude::*;
use protocol::value::{ Var32, Var64, BlockState, BlockPos };
use protocol::value::{
    ChunkSection as PtcChunkSection,
    PalettedContainer,
    PaletteFormat,
    ChunkSectionPosition
};
use protocol::packet::s2c::play::{
    S2CPlayPackets,
    SectionBlocksUpdateS2CPlayPacket,
    BlockUpdateS2CPlayPacket
};
use protocol::registry::RegEntry;


#[derive(Clone)]
pub struct ChunkSection {
    runs  : Vec<(u16, RegEntry<BlockState>)>,
    dirty : BTreeSet<u16>
}

impl ChunkSection {

    pub fn empty() -> Self {
        Self {
            runs  : vec![(4096, BLOCK_AIR)],
            dirty : BTreeSet::new()
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = RegEntry<BlockState>> {
        self.runs.iter().flat_map(|run| iter::repeat_n(run.1, run.0 as usize))
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

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

}

impl ChunkSection {

    // Sets the block state of the given run.
    pub(crate) fn overwrite_run_state(&mut self, run_index : u16, block : RegEntry<BlockState>) {
        self.runs[run_index as usize].1 = block;
        self.dirty.insert(run_index);
    }

    // Splits out all of the runs in this section.
    pub(super) fn split_out_all(&mut self) {
        self.runs = self.runs.iter().flat_map(|run| iter::repeat_n((1, run.1), run.0 as usize)).collect();
        debug_assert_eq!(self.runs.iter().map(|(len, _)| *len).sum::<u16>(), 4096);
    }

    // Makes this section as small as possible.
    pub(super) fn collapse(&mut self) {
        for i in (0..(self.runs.len() - 1)).rev() {
            // Merge runs of the same block state.
            if (self.runs[i].1 == self.runs[i + 1].1) {
                self.runs[i].0 += self.runs[i + 1].0;
                self.runs.remove(i + 1);
            }
            // Remove empty runs.
            if (self.runs[i].0 == 0) {
                self.runs.remove(i);
            }
        }
        self.runs.shrink_to_fit();
        debug_assert_eq!(self.runs.iter().map(|(len, _)| *len).sum::<u16>(), 4096);
    }

}

impl ChunkSection {

    pub(super) fn ptc_chunk_section(&self) -> PtcChunkSection {
        let mut block_count = 0;
        let block_states;
        if (self.runs.len() == 1) {
            block_count  = if (self.runs[0].1.id() != 0) { 4096 } else { 0 };
            block_states = PalettedContainer {
                bits_per_entry : 0,
                format         : PaletteFormat::SingleValued { entry : self.runs[0].1 }
            };
        } else {
            block_states = PalettedContainer {
                bits_per_entry : 15,
                format         : PaletteFormat::Direct { data : {
                    let mut data = [unsafe{ RegEntry::new_unchecked(0) }; 4096];
                    for (i, block) in self.runs.iter().flat_map(|run| iter::repeat_n(run.1, run.0 as usize)).enumerate() {
                        if (block.id() != 0) { block_count += 1; }
                        data[i] = block;
                    }
                    data
                } }
            }
        };
        PtcChunkSection {
            block_count,
            block_states,
            biomes       : PalettedContainer {
                bits_per_entry : 0,
                format         : PaletteFormat::SingleValued { entry : unsafe { RegEntry::new_unchecked(0) } }
            },
        }
    }

    pub(super) fn ptc_update_section(&self, [cx, cy, cz] : [i32; 3]) -> Option<S2CPlayPackets> {
        let dirty_len = self.dirty.len();
        if (dirty_len == 0) { None }
        else if (dirty_len == 1) {
            let linear_xyz = *self.dirty.first().unwrap();
            let [dx, dy, dz] = in_section_block_delinearise(linear_xyz);
            Some(S2CPlayPackets::BlockUpdate(BlockUpdateS2CPlayPacket {
                pos   : BlockPos {
                    x : (cx * 16) + (dx as i32),
                    y : (cy * 16) + (dy as i32),
                    z : (cz * 16) + (dz as i32)
                },
                block : self.get(linear_xyz),
            }))
        } else {
            Some(S2CPlayPackets::SectionBlocksUpdate(SectionBlocksUpdateS2CPlayPacket {
                chunk_section : ChunkSectionPosition { x : cx, y : cy, z : cz },
                blocks        : {
                    let mut blocks = Vec::with_capacity(dirty_len);
                    for &linear_xyz in &self.dirty {
                        let [dx, dy, dz] = in_section_block_delinearise(linear_xyz);
                        blocks.push(Var64::from((
                            (self.get(linear_xyz).id() as u64) << 12
                                | ((dx as u64) << 8) | ((dz as u64) << 4) | (dy as u64)
                        ) as i64));
                    }
                    blocks.into()
                },
            }))
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
