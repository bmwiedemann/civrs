//! Sound stub. The original drives AdLib/MT-32/Tandy/PC-speaker through driver overlays whose
//! music bytecode is not reproduced; this keeps the call sites so sound can be added later.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Move,
    Attack,
    CityBuilt,
    Discovery,
    Wonder,
    Menu,
    Error,
    Explosion,
}

#[derive(Default)]
pub struct Audio {
    pub enabled: bool,
}

impl Audio {
    pub fn new() -> Self {
        Audio { enabled: true }
    }
    pub fn play(&mut self, _s: Sfx) {}
    pub fn music(&mut self, _track: u8) {}
    pub fn stop_music(&mut self) {}
}
