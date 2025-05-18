use crate::conn::Connection;
use flywheelmc_common::prelude::*;
use protocol::value::Text;
use protocol::packet::s2c::play::{
    SystemChatS2CPlayPacket,
    SetTitlesAnimationS2CPlayPacket,
    SetSubtitleTextS2CPlayPacket,
    SetTitleTextS2CPlayPacket
};


#[derive(Event)]
pub struct PlayerCommsActionEvent {
    pub entity : Entity,
    pub action : PlayerCommsAction
}

pub enum PlayerCommsAction {

    Chat {
        message : Text
    },

    Actionbar {
        message : Text
    },

    Title {
        title    : Text,
        subtitle : Text,
        fade_in  : u32,
        stay     : u32,
        fade_out : u32
    }

}


pub(crate) fn handle_actions(
    mut q_conns   : Query<(&mut Connection,)>,
    mut er_action : EventReader<PlayerCommsActionEvent>
) {
    for PlayerCommsActionEvent { entity, action } in er_action.read() {
        if let Ok((mut conn,)) = q_conns.get_mut(*entity) {
            match (action) {

                PlayerCommsAction::Chat { message } => {
                    let _ = conn.send_packet_play(SystemChatS2CPlayPacket {
                        content      : message.to_nbt(),
                        is_actionbar : false
                    });
                },

                PlayerCommsAction::Actionbar { message } => {
                    let _ = conn.send_packet_play(SystemChatS2CPlayPacket {
                        content      : message.to_nbt(),
                        is_actionbar : true
                    });
                },

                PlayerCommsAction::Title { title, subtitle, fade_in, stay, fade_out } => {
                    let _ = conn.send_packet_play(SetTitlesAnimationS2CPlayPacket {
                        fade_in : *fade_in, stay : *stay, fade_out : *fade_out
                    });
                    let _ = conn.send_packet_play(SetSubtitleTextS2CPlayPacket {
                        subtitle : subtitle.to_nbt()
                    });
                    let _ = conn.send_packet_play(SetTitleTextS2CPlayPacket {
                        title : title.to_nbt()
                    });
                }

            }
        }
    }
}
