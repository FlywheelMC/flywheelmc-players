use flywheelmc_common::prelude::*;
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::c2s::play::C2SPlayPackets;


#[derive(Event)]
pub struct PacketReadEvent {
    pub entity : Entity,
    pub packet : Packet,
    pub index  : u128
}


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
#[derive(Debug)]
pub(crate) enum NextStage {
    Config,
    Play
}


pub(crate) struct PacketWriterTask {
    pub(crate) current_stage  : CurrentStage,
    pub(crate) write_receiver : mpsc::UnboundedReceiver<(SetStage, Vec<u8>,)>,
    pub(crate) stage_receiver : mpsc::UnboundedReceiver<NextStage>,
    pub(crate) stream         : OwnedWriteHalf,
    pub(crate) shutdown       : Arc<AtomicBool>,
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
                Ok((set_stage, packet,)) => {
                    match (set_stage) {
                        SetStage::NoSet  => { },
                        SetStage::Config => { match (self.current_stage) {
                            CurrentStage::Config => { },
                            CurrentStage::Startup
                                | CurrentStage::Play
                            => {
                                todo!("switch to play")
                            }
                        }; },
                        SetStage::Play => { match (self.current_stage) {
                            CurrentStage::Startup => { self.current_stage = CurrentStage::Play; },
                            CurrentStage::Config  => {
                                todo!("switch to config")
                            },
                            CurrentStage::Play => { }
                        } }
                    }
                    match (task::timeout(self.send_timeout, self.stream.write_all(&packet)).await) {
                        Ok(Ok(_)) => { },
                        Ok(Err(err)) => {
                            // TODO: Log warning
                            self.shutdown.store(true, AtomicOrdering::Relaxed);
                            return Err(());
                        }
                        Err(_) => {
                            // TODO: Log warning (timed out)
                            self.shutdown.store(true, AtomicOrdering::Relaxed);
                            return Err(());
                        }
                    }
                },
                Err(mpsc::TryRecvError::Empty) => { },
                Err(mpsc::TryRecvError::Disconnected) => { return Err(()); }
            }
            match (self.stage_receiver.try_recv()) {
                Ok(stage) => { self.current_stage =  match (stage) {
                    NextStage::Config => CurrentStage::Config,
                    NextStage::Play   => CurrentStage::Play,
                }; },
                Err(mpsc::TryRecvError::Empty) => { },
                Err(mpsc::TryRecvError::Disconnected) => { return Err(()); }
            }
            task::yield_now().await;
        }
    }

}
