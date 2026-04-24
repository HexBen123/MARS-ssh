#[derive(Debug, Clone)]
pub struct PortPool {
    min: u16,
    max: u16,
    used: std::collections::BTreeSet<u16>,
    cursor: u16,
}

impl PortPool {
    pub fn new(min: u16, max: u16) -> Self {
        Self {
            min,
            max,
            used: std::collections::BTreeSet::new(),
            cursor: min,
        }
    }

    pub fn reserve(&mut self, port: u16) -> bool {
        if port < self.min || port > self.max || self.used.contains(&port) {
            return false;
        }
        self.used.insert(port);
        true
    }

    pub fn allocate(&mut self) -> anyhow::Result<u16> {
        let size = self.max - self.min + 1;
        for i in 0..size {
            let offset = (self.cursor - self.min + i) % size;
            let port = self.min + offset;
            if !self.used.contains(&port) {
                self.used.insert(port);
                self.cursor = if port == self.max { self.min } else { port + 1 };
                return Ok(port);
            }
        }
        anyhow::bail!("port pool exhausted ({}-{})", self.min, self.max)
    }

    pub fn release(&mut self, port: u16) {
        self.used.remove(&port);
    }
}
