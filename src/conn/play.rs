use flywheelmc_common::prelude::*;
use crate::conn::Connection;
use crate::conn::packet::{ PacketReadEvent, NextStage, Packet };
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::c2s::play::C2SPlayPackets;


#[derive(Component)]
pub(crate) struct ConnStatePlay {
    pub(crate) stage : NextStage
}


pub(crate) fn handle_state(
    mut q_conns   : Query<(Entity, &mut Connection, &mut ConnStatePlay),>,
    mut ew_packet : EventWriter<PacketReadEvent>
) {
    for (entity, mut conn, mut state) in &mut q_conns {
        match (state.stage) {

            NextStage::Config => {
                if let Some(packet) = conn.read_packet() {
                    if let C2SConfigPackets::FinishConfiguration(_) = packet {
                        state.stage = NextStage::Play;
                        if (conn.stage_sender.send(NextStage::Play).is_err()) {
                            error!("Failed to switch peer {} to play stage", conn.peer_addr);
                            conn.kick("Could not switch to play stage");
                        }
                        trace!("Switched peer {} to play stage", conn.peer_addr);
                    } else {
                        ew_packet.write(PacketReadEvent {
                            entity,
                            packet : Packet::Config(packet),
                            index  : conn.packet_index.increment()
                        });
                    }
                }
            },

            NextStage::Play => {
                if let Some(packet) = conn.read_packet() {
                    if let C2SPlayPackets::ConfigurationAcknowledged(_) = packet {
                        state.stage = NextStage::Config;
                        if (conn.stage_sender.send(NextStage::Config).is_err()) {
                            error!("Failed to switch peer {} to config stage", conn.peer_addr);
                            conn.kick("Could not switch to config stage");
                        }
                        trace!("Switched peer {} to config stage", conn.peer_addr);
                    } else {
                        ew_packet.write(PacketReadEvent {
                            entity,
                            packet : Packet::Play(packet),
                            index  : conn.packet_index.increment()
                        });
                    }
                }
            }

        }
    }
}
