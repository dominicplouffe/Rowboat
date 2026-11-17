use super::*;
pub struct ChSession;
impl ChSession {
    pub async fn connect(_params: ConnectParams) -> DbResult<Self> {
        Err(DbError::other("not implemented"))
    }
}
#[async_trait]
impl Session for ChSession {
    fn kind(&self) -> DbKind { DbKind::Sqlite }
    fn server_version(&self) -> String { String::new() }
    async fn execute(&self, _sql: &str, _options: ExecOptions) -> DbResult<QueryResult> { unimplemented!() }
    async fn cancel(&self) {}
    async fn commit(&self) -> DbResult<()> { Ok(()) }
    async fn rollback(&self) -> DbResult<()> { Ok(()) }
    fn in_transaction(&self) -> bool { false }
    fn supports_transactions(&self) -> bool { false }
    fn set_auto_commit(&self, _auto_commit: bool) {}
    async fn children(&self, _path: &[String]) -> DbResult<Vec<SchemaNode>> { Ok(vec![]) }
    async fn ddl(&self, _path: &[String]) -> DbResult<String> { Ok(String::new()) }
    async fn close(&self) {}
}
