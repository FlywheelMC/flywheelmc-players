use flywheelmc_common::prelude::*;
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::c2s::play::C2SPlayPackets;


#[derive(Event)]
pub struct PacketReadEvent {
    pub entity : Entity,
    pub packet : Packet,
    pub index  : u128
}


#[expect(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Packet {
    Config(C2SConfigPackets),
    Play(C2SPlayPackets)
}

impl From<C2SConfigPackets> for Packet {
    fn from(value : C2SConfigPackets) -> Self {
        Self::Config(value)
    }
}

impl From<C2SPlayPackets> for Packet {
    fn from(value : C2SPlayPackets) -> Self {
        Self::Play(value)
    }
}

#[derive(Debug)]
pub(crate) enum SetStage {
    NoSet,
    Config,
    Play
}
#[derive(Debug)]
pub(crate) enum CurrentStage {
    Startup,
    Config,
    Play
}
#[derive(Debug, PartialEq)]
pub(crate) enum NextStage {
    Config,
    Play
}


pub(crate) struct PacketWriterTask {
    pub(crate) peer_addr      : SocketAddr,
    pub(crate) current_stage  : CurrentStage,
    pub(crate) write_receiver : channel::Receiver<(ShortName<'static>, SetStage, Vec<u8>,)>,
    pub(crate) stage_receiver : channel::Receiver<NextStage>,
    pub(crate) close_sender   : channel::Sender<Cow<'static, str>>,
    pub(crate) stream         : TcpStream,
    pub(crate) send_timeout   : Duration
}

impl PacketWriterTask {

    #[inline(always)]
    pub(crate) async fn run(self) -> () {
        let _ = self.run_inner().await;
    }

    async fn run_inner(mut self) -> Result<(), ()> {
        loop {
            match (self.write_receiver.try_recv()) {
                Ok((packet_type, set_stage, packet,)) => {
                    trace!("Sending packet ({}) {} to peer {}...", packet.len(), packet_type, self.peer_addr);
                    match (set_stage) {
                        SetStage::NoSet  => { },
                        SetStage::Config => { match (self.current_stage) {
                            CurrentStage::Config => { },
                            CurrentStage::Startup
                                | CurrentStage::Play
                            => {
                                todo!("switch to config")
                            }
                        }; },
                        SetStage::Play => { match (self.current_stage) {
                            CurrentStage::Startup => { self.current_stage = CurrentStage::Play; },
                            CurrentStage::Config  => {
                                todo!("switch to play")
                            },
                            CurrentStage::Play => { }
                        } }
                    }
                    match (task::timeout(self.send_timeout, async {
                        self.stream.write_all(&packet).await?;
                        self.stream.flush().await
                    }).await) {
                        Some(Ok(_)) => { },
                        Some(Err(err)) => {
                            error!("Failed to send packet to peer {}: {}", self.peer_addr, err);
                            let _ = self.close_sender.send(Cow::Owned(err.to_string()));
                            return Err(());
                        }
                        None => {
                            error!("Failed to send packet to peer {}: {}", self.peer_addr, io::ErrorKind::TimedOut);
                            let _ = self.close_sender.send(Cow::Borrowed("timed out"));
                            return Err(());
                        }
                    }
                },
                Err(channel::TryRecvError::Empty) => { },
                Err(channel::TryRecvError::Closed) => { return Err(()); }
            }
            match (self.stage_receiver.try_recv()) {
                Ok(stage) => { self.current_stage =  match (stage) {
                    NextStage::Config => CurrentStage::Config,
                    NextStage::Play   => CurrentStage::Play,
                }; },
                Err(channel::TryRecvError::Empty) => { },
                Err(channel::TryRecvError::Closed) => { return Err(()); }
            }
            task::yield_now().await;
        }
    }

}
