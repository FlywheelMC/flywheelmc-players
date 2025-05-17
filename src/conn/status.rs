use crate::{
    ServerMotd,
    ServerVersion,
    ServerFavicon
};
use crate::conn::Connection;
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
    mut q_conns   : Query<(&mut Connection, &mut ConnStateStatus),>,
        r_motd    : Res<ServerMotd>,
        r_version : Res<ServerVersion>,
        r_favicon : Res<ServerFavicon>
) {
    for (mut conn, mut state) in &mut q_conns {
        if let Some(packet) = conn.read_packet() {
            match (packet) {

                C2SStatusPackets::StatusRequest(StatusRequestC2SStatusPacket) => {
                    if (state.sent_status) {
                        conn.close();
                    } else {
                        trace!("Peer {} requested status", conn.peer_addr);
                        state.sent_status = true;
                        if (unsafe { conn.send_packet_noset(StatusResponse {
                            version              : StatusResponseVersion {
                                name     : r_version.0.to_string(),
                                protocol : PROTOCOL_VERSION
                            },
                            players              : None,
                            desc                 : r_motd.0.clone(),
                            favicon_png_b64      : r_favicon.0.to_string(),
                            enforce_chat_reports : false,
                            prevent_chat_reports : true
                        }.to_packet()) }.is_err()) { continue; }
                    }
                },

                C2SStatusPackets::PingRequest(PingRequestC2SStatusPacket { timestamp }) => {
                    if (state.sent_pong) {
                        conn.close();
                    } else {
                        trace!("Peer {} requested ping", conn.peer_addr);
                        state.sent_pong = true;
                        if (unsafe { conn.send_packet_noset(
                            PongResponseS2CStatusPacket { timestamp }
                        ) }.is_err()) { continue; }
                    }
                }

            }
            if (state.sent_status && state.sent_pong) {
                conn.close();
            }
        }
    }
}
