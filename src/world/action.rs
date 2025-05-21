use crate::world::{ World, SetBlockBatch };
use flywheelmc_common::prelude::*;
use protocol::value::{ Identifier, BlockState };
use protocol::registry::RegEntry;


#[derive(Event)]
#[non_exhaustive]
pub struct WorldChunkLoading {
    pub entity : Entity,
    pub pos    : Vec2<i32>
}


#[derive(Event)]
pub struct WorldChunkActionEvent {
    pub entity : Entity,
    pub action : WorldChunkAction
}

pub enum WorldChunkAction {

    MarkReady {
        chunk_pos : Vec2<i32>
    },

    Set {
        #[expect(clippy::type_complexity)]
        blocks : Vec<(Vec3<i64>, String, Vec<(String, String,)>)>
    }

}


pub(crate) fn handle_actions(
    mut q_worlds  : Query<(&mut World,)>,
    mut er_action : EventReader<WorldChunkActionEvent>
) {
    for WorldChunkActionEvent { entity, action } in er_action.read() {
        if let Ok((mut world,)) = q_worlds.get_mut(*entity) {
            match (action) {

                WorldChunkAction::MarkReady { chunk_pos } => {
                    if let Some(chunk) = world.chunks.get_mut(chunk_pos)
                        && (! chunk.ready)
                    {
                        chunk.ready = true;
                        world.ready_chunks.push_back(*chunk_pos);
                    }
                },

                WorldChunkAction::Set { blocks } => {
                    let mut batch = SetBlockBatch::new(&mut world);

                    for (block_pos, block_id, states,) in blocks {
                        let chunk_pos = Vec2::new((block_pos.x / 16) as i32, (block_pos.z / 16) as i32);
                        if (batch.is_chunk_loaded(chunk_pos)) {
                            let block_id = Identifier::from(block_id);
                            if let Some(mut block_state) = BlockState::default_for(&block_id) {
                                for (state, value,) in states {
                                    if (block_state.properties.contains_key(state)) {
                                        block_state.properties.insert(state.clone(), value.clone());
                                    }
                                }
                                if let Some(block_id) = block_state.to_id() {
                                    let block = unsafe { RegEntry::new_unchecked(block_id as u32) };
                                    batch.set((block_pos.x, block_pos.y as u16, block_pos.z), block);
                                }
                            }
                        }
                    }
                }

            }
        }
    }
}
