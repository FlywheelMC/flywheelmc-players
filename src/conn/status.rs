use crate::{
    ServerMotd,
    ServerVersion,
    ServerFavicon
};
use crate::conn::ConnStream;
use flywheelmc_common::prelude::*;
use protocol::PROTOCOL_VERSION;
use protocol::packet::c2s::status::{
    C2SStatusPackets,
    StatusRequestC2SStatusPacket,
    PingRequestC2SStatusPacket
};
use protocol::packet::s2c::status::{
    StatusResponse,
    StatusResponseVersion,
    PongResponseS2CStatusPacket
};


#[derive(Component, Default)]
#[component(storage = "SparseSet")]
pub(crate) struct ConnStateStatus {
    sent_status : bool,
    sent_pong   : bool
}


pub(crate) fn handle_state(
    mut cmds      : Commands,
    mut q_conns   : Query<(&mut ConnStream, &mut ConnStateStatus),>,
        r_motd    : Res<ServerMotd>,
        r_version : Res<ServerVersion>,
        r_favicon : Res<ServerFavicon>
) {
    for (mut conn_stream, mut state) in &mut q_conns {
        if let Some(packet) = conn_stream.read_packet() {
            match (packet) {

                C2SStatusPackets::StatusRequest(StatusRequestC2SStatusPacket) => {
                    if (state.sent_status) {
                        conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                    } else {
                        state.sent_status = true;
                        conn_stream.send_packet(&mut cmds, StatusResponse {
                            version              : StatusResponseVersion {
                                name     : r_version.0.to_string(),
                                protocol : PROTOCOL_VERSION
                            },
                            players              : None,
                            desc                 : r_motd.0.clone(),
                            favicon_png_b64      : r_favicon.0.to_string(),
                            enforce_chat_reports : false,
                            prevent_chat_reports : true
                        }.to_packet());
                    }
                },

                C2SStatusPackets::PingRequest(PingRequestC2SStatusPacket { timestamp }) => {
                    if (state.sent_pong) {
                        conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                    } else {
                        state.sent_pong = true;
                        conn_stream.send_packet(&mut cmds, PongResponseS2CStatusPacket { timestamp });
                    }
                }

            }
            if (state.sent_status && state.sent_pong) {
                conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
            }
        }
    }
}
