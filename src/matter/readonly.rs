//! Device workers only ever get a `ReadOnlyNode`, so the dashboard cannot
//! write attributes or invoke commands by accident.

use matter_controller::{AttributePath, Error, Node, ReadPath, Subscription, Value};

#[derive(Clone)]
pub struct ReadOnlyNode(Node);

impl ReadOnlyNode {
    pub fn new(node: Node) -> Self {
        Self(node)
    }

    pub fn node_id(&self) -> u64 {
        self.0.node_id()
    }

    pub async fn read(&self, paths: &[ReadPath]) -> Result<Vec<(AttributePath, Value)>, Error> {
        self.0.read(paths).await
    }

    pub async fn subscribe(
        &self,
        paths: &[ReadPath],
        min_interval_s: u16,
        max_interval_s: u16,
    ) -> Result<Subscription, Error> {
        self.0.subscribe(paths, &[], min_interval_s, max_interval_s).await
    }
}
