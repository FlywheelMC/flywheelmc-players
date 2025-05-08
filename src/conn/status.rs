use crate::ServerMotd;
use crate::conn::ConnStream;
use flywheelmc_common::prelude::*;
use voxidian_protocol::{ MINECRAFT_VERSION, PROTOCOL_VERSION };
use voxidian_protocol::packet::c2s::status::{
    C2SStatusPackets,
    StatusRequestC2SStatusPacket,
    PingRequestC2SStatusPacket
};
use voxidian_protocol::packet::s2c::status::{
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
    mut cmds    : Commands,
    mut q_conns : Query<(&mut ConnStream, &mut ConnStateStatus),>,
        r_motd  : Res<ServerMotd>
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
                                name     : format!("LighthouseMC {}", MINECRAFT_VERSION),
                                protocol : PROTOCOL_VERSION
                            },
                            players              : None,
                            desc                 : r_motd.0.clone(),
                            favicon_png_b64      : "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAMAAACdt4HsAAAABGdBTUEAALGPC/xhBQAAAAFzUkdCAK7OHOkAAAA8UExURUdwTP748/////////////////////////+/Af8BAVMTaQgADDsPSSQFL/jSvtajp/9NTf/GTIV+iXJXgEo/9IwAAAAHdFJOUwD80BaaZz7Q6pnNAAAC0klEQVRYw91X2XKDMAysANscvoD//9dKvm1IGtOXTtWG6TTserWSbPj6+uMxYvwGPoEQMD3HD8LF8FQE4s15aiEeamCIXzGMEOxRAiD06kI/S2IScHqC45EEFGDWNUl44kAUsEqUwB+UQK9rltBdiFGkDCQVAsb+DNL6cj2h28acgaTf7hxSBtJ9JObwOANHcfbWYaoywIDOHIZYg4CXnb2EFhzJQEfQWchkQRTQa0K0IOHJBPbIghRdJmQLIlyprk7gQpwlXimpcCLfuMgmdudhhquLCRUEV6y+9YNQwOlTjwNumEVZJtq7Lx5WcKl0c0u5WQ8Cqj5BBeeaxPtL7SJueFAQIqDqE6SHI8JdbKp2EZPWNUE1LHSkaVXHhi6WSYOpCXTd61gHGZaOP6ooA50ZDcFe1QFPRVCbg+UoCJDfNgRzaSNVxWwxgoIiBRKwtAR7URYkM2orojERHbAXghld4InAJOQWmTKAk4ClrAIyzvMO6QxFB2S1Pkbe1MggSwRT6flMEiInLy0IFHlLQrxZlqWcDQTsnmFK9xwVw3ak79BhTGCxZeuRq0iAlQh3jS3DkfIL+MU0zU85kA1iGoNPYJIPuKUG/Eh4SwT1bDGfg9Pg7+Q0D+ZADnmY9F967PJ4W28AMQfnA7AxrIUBAHR1ukYGQT8JqLd5LOQeGGg5R86n+Jjn02JOlMfb9qTKEpwRSOElcwz/F8GD/BsBztwgwYugdVm4Z2Reiw7L3wjwW8w8ZwqI6mMekOF3Alwh9DzfcDgntbFLib87poaGAd3YKRnElmDXQ7enFHVfw+D6Qi9NGHjx5Eo26J8JLLw86fmV4UqA68PLg55D68OFgDqVv3/FgP01gdU/vX64lxS93xNYDR+8vtDAQKIoCBwcPnhMcW9akSMSWKPTUH5KQf3nG0n74fgUHmYvN7Lv5oF1Pq+PHGdwoGHCC+O/ef389/ENv2s5bHHprKEAAAAASUVORK5CYII=".to_string(),
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
