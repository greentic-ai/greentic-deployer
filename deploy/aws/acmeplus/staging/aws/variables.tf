variable "aws_region" {
  type = string
  default = "us-west-2"
}

variable "otel_exporter_otlp_endpoint" {
  type = string
  default = "https://otel.greentic.ai"
}

# Secrets resolved via greentic-secrets
variable "slack_bot_token_secret_id" {
  type = string
  description = "Secret identifier for SLACK_BOT_TOKEN"
}

variable "crm_api_token_secret_id" {
  type = string
  description = "Secret identifier for CRM_API_TOKEN"
}

variable "pagerduty_api_key_secret_id" {
  type = string
  description = "Secret identifier for PAGERDUTY_API_KEY"
}

variable "teams_client_secret_secret_id" {
  type = string
  description = "Secret identifier for TEAMS_CLIENT_SECRET"
}

