use crate::world::{ World, in_section_block_linearise };
use flywheelmc_common::prelude::*;
use protocol::value::BlockState;
use protocol::registry::RegEntry;


pub(super) struct SetBlockBatch<'l> {
    world              : &'l mut World,
    chunks_to_collapse : BTreeSet<(i32, u8, i32,)>
}

impl<'l> SetBlockBatch<'l> {

    pub(super) fn new(world : &'l mut World) -> Self { Self {
        world,
        chunks_to_collapse : BTreeSet::new()
    } }

    pub(super) fn is_chunk_loaded(&self, cpos : Vec2<i32>) -> bool {
        self.world.chunks.contains_key(&cpos)
    }


    pub(super) fn set(&mut self, (x, y, z) : (i64, u16, i64,), block : RegEntry<BlockState>) {
        let cpos = Vec2::new(x.div_floor(16) as i32, z.div_floor(16) as i32);
        let cy   = y.div_floor(16) as u8;
        let dx = x.rem_euclid(16) as u8;
        let dy = y.rem_euclid(16) as u8;
        let dz = z.rem_euclid(16) as u8;
        let Some(chunk) = self.world.chunks.get_mut(&cpos)
            else { return; };
        let Some(section) = chunk.sections.get_mut(cy as usize)
            else { return; };
        if (self.chunks_to_collapse.insert((cpos.x, cy, cpos.y,))) {
            section.split_out_all();
        }
        section.overwrite_run_state(in_section_block_linearise(dx, dy, dz), block);
    }

}

impl<'l> Drop for SetBlockBatch<'l> {
    fn drop(&mut self) {
        for &(cx, cy, cz) in &self.chunks_to_collapse {
            if let Some(chunk) = self.world.chunks.get_mut(&Vec2::new(cx, cz))
                && let Some(section) = chunk.sections.get_mut(cy as usize)
            { section.collapse(); }
        }
    }
}
