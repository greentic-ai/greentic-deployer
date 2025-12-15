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
  nats_cluster = "nats-staging-acmeplus"
  nats_admin_url = "https://nats.staging.acmeplus.svc"
  telemetry_endpoint = "https://otel.greentic.ai"
}

resource "aws_ecs_cluster" "nats" {
  name = local.nats_cluster
}

resource "aws_ecs_task_definition" "runner_greentic_acme_plus_component" {
  family = "greentic.acme.plus.component"
  cpu = "512"
  memory = "1024"
  requires_compatibilities = ["FARGATE"]
  network_mode = "awsvpc"
  container_definitions = <<EOF
[ {
  "name": "greentic.acme.plus.component",
  "image": "greentic/runner:latest",
  "environment": [
    { "name": "NATS_URL", "value": "https://nats.staging.acmeplus.svc" },
    { "name": "OTEL_EXPORTER_OTLP_ENDPOINT", "value": "https://otel.greentic.ai" },
    { "name": "OTEL_RESOURCE_ATTRIBUTES", "value": "deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-aws" }
  ]
} ]
EOF
}

resource "aws_ecs_service" "runner_greentic_acme_plus_component_service" {
  name = "greentic.acme.plus.component-service"
  cluster = aws_ecs_cluster.nats.id
  task_definition = aws_ecs_task_definition.runner_greentic_acme_plus_component.arn
  desired_count = 2
}

