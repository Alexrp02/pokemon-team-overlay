#[derive(Clone, Debug, Default)]
pub struct UiSnapshot {
    pub local_ticket: String,
    pub status: String,
    pub local_urls: Vec<TeamUrl>,
    pub remote_urls: Vec<TeamUrl>,
}

#[derive(Clone, Debug)]
pub struct TeamUrl {
    pub team_name: String,
    pub url: String,
}
