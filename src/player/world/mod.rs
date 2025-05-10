use flywheelmc_common::prelude::*;


#[derive(Component)]
pub struct ChunkCentre(Dirty<(i32, i32,)>);
