pub(crate) enum AgentOverviewAction {
    Select(String),
    StartAdd,
    CancelAdd,
    SaveAdd {
        endpoint: String,
        node: String,
        service: String,
    },
    RequestRemove(String),
    ConfirmRemove(String),
    CancelRemove,
    SaveListener(String),
    RequestRemoveListener(String),
    ConfirmRemoveListener(String),
    CancelRemoveListener,
}
