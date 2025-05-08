use crate::conn::ConnStream;
use crate::conn::status::ConnStateStatus;
use crate::conn::login::ConnStateLogin;
use flywheelmc_common::prelude::*;
use voxidian_protocol::packet::c2s::handshake::{
    IntentionC2SHandshakePacket,
    IntendedStage
};


#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct ConnStateHandshake;


pub(crate) fn handle_state(
    mut cmds    : Commands,
    mut q_conns : Query<(Entity, &mut ConnStream,), (With<ConnStateHandshake>,)>
) {
    for (entity, mut conn_stream,) in &mut q_conns {
        if let Some(packet) = conn_stream.read_packet() {
            let IntentionC2SHandshakePacket { intended_stage, .. } = packet;

            let mut entity = cmds.entity(entity);
            entity.remove::<ConnStateHandshake>();
            match (intended_stage) {

                IntendedStage::Status => { entity.insert(ConnStateStatus::default()); },

                IntendedStage::Login | IntendedStage::Transfer => {
                    // TODO: Check protocol_version
                    entity.insert(ConnStateLogin::WaitingForHello);
                }

            }
        }
    }
}
