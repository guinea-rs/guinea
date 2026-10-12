use guinea::prelude::reducer;
use guinea_widgets::chart::RingSeries;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Metrics {
    pub cpu: RingSeries,
    pub memory: RingSeries,
}

#[derive(Clone)]
pub enum Sampled {
    At { at: u64, cpu: f32, memory: f32 },
}

#[reducer]
fn metrics(this: &mut Metrics, update: Sampled) {
    match update {
        Sampled::At { at, cpu, memory } => {
            this.cpu.push((at, cpu));
            this.memory.push((at, memory));
        }
    }
}
