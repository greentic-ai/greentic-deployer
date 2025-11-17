# Terraform snippet for tenant `acmeplus` in `staging`
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}

provider "aws" {
  region = "us-west-2"
}

locals {
  nats_cluster = "default"
  nats_admin_url = "https://nats.staging.acmeplus.svc"
  telemetry_endpoint = "https://otel.greentic.ai"
}


# Secret data sources (values resolved during apply via greentic-secrets)
data "aws_secretsmanager_secret_version" "secret_slack_bot_token" {
  secret_id = var.slack_bot_token_secret_id
}

data "aws_secretsmanager_secret_version" "secret_crm_api_token" {
  secret_id = var.crm_api_token_secret_id
}

data "aws_secretsmanager_secret_version" "secret_pagerduty_api_key" {
  secret_id = var.pagerduty_api_key_secret_id
}

data "aws_secretsmanager_secret_version" "secret_teams_client_secret" {
  secret_id = var.teams_client_secret_secret_id
}

resource "aws_ecs_cluster" "nats" {
  name = local.nats_cluster
}

resource "aws_ecs_task_definition" "runner_greentic_acme_plus_runner" {
  family = "greentic.acme.plus-runner"
  cpu = "512"
  memory = "1024"
  requires_compatibilities = ["FARGATE"]
  network_mode = "awsvpc"
  container_definitions = <<EOF
[ {
  "name": "greentic.acme.plus-runner",
  "image": "greentic/runner:latest",
  "environment": [
    { "name": "NATS_URL", "value": "https://nats.staging.acmeplus.svc" },
    { "name": "OTEL_EXPORTER_OTLP_ENDPOINT", "value": "https://otel.greentic.ai" },
    { "name": "OTEL_RESOURCE_ATTRIBUTES", "value": "deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-aws" },
    { "name": "SLACK_BOT_TOKEN", "value": data.aws_secretsmanager_secret_version.secret_slack_bot_token.secret_string },
    { "name": "CRM_API_TOKEN", "value": data.aws_secretsmanager_secret_version.secret_crm_api_token.secret_string },
    { "name": "PAGERDUTY_API_KEY", "value": data.aws_secretsmanager_secret_version.secret_pagerduty_api_key.secret_string },
    { "name": "TEAMS_CLIENT_SECRET", "value": data.aws_secretsmanager_secret_version.secret_teams_client_secret.secret_string }
  ]
} ]
EOF
}

resource "aws_ecs_service" "runner_greentic_acme_plus_runner_service" {
  name = "greentic.acme.plus-runner-service"
  cluster = aws_ecs_cluster.nats.id
  task_definition = aws_ecs_task_definition.runner_greentic_acme_plus_runner.arn
  desired_count = 1
}


# Channel ingress endpoints
# - Slack Support (type = channels.slack.support, oauth_required = false)
#   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/channels.slack.support
# - Teams Ops (type = channels.teams.ops, oauth_required = false)
#   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/channels.teams.ops
# - messaging.subjects.ops.alerts (type = messaging.subjects.ops.alerts, oauth_required = false)
#   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/messaging.subjects.ops.alerts
# - messaging.subjects.support.inbound (type = messaging.subjects.support.inbound, oauth_required = false)
#   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/messaging.subjects.support.inbound

# OAuth redirect URLs
# - /oauth/microsoft/callback via /oauth/microsoft/callback/acmeplus/staging
# - /oauth/slack/callback via /oauth/slack/callback/acmeplus/staging
