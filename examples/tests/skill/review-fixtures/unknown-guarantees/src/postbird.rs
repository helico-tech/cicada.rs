// A thin client of the Postbird HTTP API. The vendor's documentation is not in this repository.

pub struct Postbird {
    pub http: reqwest::Client,
    pub api_key: String,
}

impl Postbird {
    /// Posts to /v2/messages with `client_reference` in the body.
    pub async fn send_template(
        &self,
        template: &str,
        to: &str,
        fields: &[(&str, &str)],
        client_reference: &str,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "template": template,
            "to": to,
            "fields": fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>(),
            "client_reference": client_reference,
        });
        self.http
            .post("https://api.postbird.example/v2/messages")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
