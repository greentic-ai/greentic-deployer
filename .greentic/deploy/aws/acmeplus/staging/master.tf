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

locals {
  service_sg_ids = length(var.security_group_ids) > 0 ? var.security_group_ids : (var.vpc_id != "" && length(aws_security_group.greentic_default) > 0 ? [aws_security_group.greentic_default[0].id] : [])
}

resource "aws_security_group" "greentic_default" {
  count = var.vpc_id != "" && length(var.security_group_ids) == 0 ? 1 : 0
  name        = "greentic-deployer"
  description = "Greentic runner default egress"
  vpc_id      = var.vpc_id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
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
  ],
  "logConfiguration": {
    "logDriver": "awslogs",
    "options": {
      "awslogs-group": "/greentic/acmeplus/staging/greentic_acme_plus_component",
      "awslogs-region": var.aws_region,
      "awslogs-stream-prefix": "greentic.acme.plus.component"
    }
  }
} ]
EOF
}

resource "aws_ecs_service" "runner_greentic_acme_plus_component_service" {
  name = "greentic.acme.plus.component-service"
  cluster = aws_ecs_cluster.nats.id
  task_definition = aws_ecs_task_definition.runner_greentic_acme_plus_component.arn
  desired_count = 2
  network_configuration {
    subnets = var.subnet_ids
    security_groups = local.service_sg_ids
    assign_public_ip = var.assign_public_ip
  }
}

resource "aws_cloudwatch_log_group" "runner_greentic_acme_plus_component_logs" {
  name = "/greentic/acmeplus/staging/greentic_acme_plus_component"
  retention_in_days = var.log_retention_days
}

resource "aws_appautoscaling_target" "runner_greentic_acme_plus_component_as" {
  max_capacity       = 2 + var.autoscaling_max_extra
  min_capacity       = 2
  resource_id        = "service/${aws_ecs_cluster.nats.name}/${aws_ecs_service.runner_greentic_acme_plus_component_service.name}"
  scalable_dimension = "ecs:service:DesiredCount"
  service_namespace  = "ecs"
}

resource "aws_appautoscaling_policy" "runner_greentic_acme_plus_component_cpu" {
  name               = "greentic.acme.plus.component-cpu-policy"
  policy_type        = "TargetTrackingScaling"
  resource_id        = aws_appautoscaling_target.runner_greentic_acme_plus_component_as.resource_id
  scalable_dimension = aws_appautoscaling_target.runner_greentic_acme_plus_component_as.scalable_dimension
  service_namespace  = aws_appautoscaling_target.runner_greentic_acme_plus_component_as.service_namespace

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageCPUUtilization"
    }
    target_value = var.autoscaling_cpu_target
  }
}

