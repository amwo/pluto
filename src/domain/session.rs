use uuid::Uuid;

use crate::domain::Mode;

#[derive(Clone, Copy, Debug)]
pub struct Session {
    pub id: Uuid,
    pub mode: Mode,
}

impl Session {
    pub fn new(mode: Mode) -> Self {
        Self {
            id: Uuid::now_v7(),
            mode,
        }
    }
}
