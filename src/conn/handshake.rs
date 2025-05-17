use crate::conn::{ Connection, RealStage };
use crate::conn::status::ConnStateStatus;
use crate::conn::login::ConnStateLogin;
use flywheelmc_common::prelude::*;
use protocol::packet::c2s::handshake::{
    IntentionC2SHandshakePacket,
    IntendedStage
};


#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct ConnStateHandshake;


pub(crate) fn handle_state(
    mut cmds    : Commands,
    mut q_conns : Query<(Entity, &mut Connection,), (With<ConnStateHandshake>,)>
) {
    for (entity, mut conn,) in &mut q_conns {
        if let Some(packet) = conn.read_packet() {
            let IntentionC2SHandshakePacket { intended_stage, .. } = packet;

            let mut entity = cmds.entity(entity);
            entity.remove::<ConnStateHandshake>();
            match (intended_stage) {

                IntendedStage::Status => {
                    conn.real_stage = RealStage::Status;
                    entity.insert(ConnStateStatus::default());
                },

                IntendedStage::Login | IntendedStage::Transfer => {
                    conn.real_stage = RealStage::Login;
                    // TODO: Check protocol_version
                    entity.insert(ConnStateLogin::WaitingForHello);
                }

            }
        }
    }
}
