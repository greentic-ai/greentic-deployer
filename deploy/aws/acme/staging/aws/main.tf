# Terraform snippet for tenant `acme` in `staging`
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}

provider "aws" {
  region = "us-west-2"
}

locals {
  nats_cluster = "nats-staging-acme"
  nats_admin_url = "https://nats.staging.acme.svc"
  telemetry_endpoint = "https://otel.greentic.ai"
}


# Secret data sources (values resolved during apply via greentic-secrets)
data "aws_secretsmanager_secret_version" "secret_slack_bot_token" {
  secret_id = var.slack_bot_token_secret_id
}

data "aws_secretsmanager_secret_version" "secret_teams_client_secret" {
  secret_id = var.teams_client_secret_secret_id
}

resource "aws_ecs_cluster" "nats" {
  name = local.nats_cluster
}

resource "aws_ecs_task_definition" "runner_greentic_acme_runner" {
  family = "greentic.acme-runner"
  cpu = "512"
  memory = "1024"
  requires_compatibilities = ["FARGATE"]
  network_mode = "awsvpc"
  container_definitions = <<EOF
[ {
  "name": "greentic.acme-runner",
  "image": "greentic/runner:latest",
  "environment": [
    { "name": "NATS_URL", "value": "https://nats.staging.acme.svc" },
    { "name": "OTEL_EXPORTER_OTLP_ENDPOINT", "value": "https://otel.greentic.ai" },
    { "name": "OTEL_RESOURCE_ATTRIBUTES", "value": "deployment.environment=staging,greentic.tenant=acme,service.name=greentic-deployer-aws" },
    { "name": "SLACK_BOT_TOKEN", "value": data.aws_secretsmanager_secret_version.secret_slack_bot_token.secret_string },
    { "name": "TEAMS_CLIENT_SECRET", "value": data.aws_secretsmanager_secret_version.secret_teams_client_secret.secret_string }
  ]
} ]
EOF
}

resource "aws_ecs_service" "runner_greentic_acme_runner_service" {
  name = "greentic.acme-runner-service"
  cluster = aws_ecs_cluster.nats.id
  task_definition = aws_ecs_task_definition.runner_greentic_acme_runner.arn
  desired_count = 1
}


# OAuth redirect URLs
# - /oauth/microsoft/callback via /oauth/microsoft/callback/acme/staging
# - /oauth/slack/callback via /oauth/slack/callback/acme/staging
