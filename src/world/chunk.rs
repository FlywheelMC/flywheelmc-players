use crate::world::ChunkSection;
use flywheelmc_common::prelude::*;
use protocol::value::ChunkSectionData as PtcChunkSectionData;


pub struct Chunk {
    pub(super) sections : Vec<ChunkSection>
}


impl Chunk {

    pub(crate) fn ptc_chunk_section_data(&self) -> PtcChunkSectionData {
        let mut sections = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            sections.push(section.ptc_chunk_section());
        }
        PtcChunkSectionData { sections }
    }

}
