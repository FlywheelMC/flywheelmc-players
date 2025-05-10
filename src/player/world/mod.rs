use flywheelmc_common::prelude::*;
use protocol::value::BlockState;
use protocol::registry::RegEntry;


mod chunk_section;
pub use chunk_section::*;


const BLOCK_AIR : RegEntry<BlockState> = unsafe { RegEntry::new_unchecked(0) };


#[derive(Component)]
pub struct ChunkCentre(Dirty<Vec2<i32>>);

#[derive(Component)]
pub struct ViewDistance(Dirty<u8>);

#[derive(Component)]
pub struct Chunk {
    pos      : Vec2<i32>,
    sections : Vec<ChunkSection>
}
