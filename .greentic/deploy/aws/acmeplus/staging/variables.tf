variable "aws_region" {
  type = string
  default = "us-west-2"
}

variable "otel_exporter_otlp_endpoint" {
  type = string
  default = "https://otel.greentic.ai"
}

variable "subnet_ids" {
  type = list(string)
  description = "Subnets for ECS tasks"
  default = []
}

variable "security_group_ids" {
  type = list(string)
  description = "Security groups for ECS services"
  default = []
}

variable "vpc_id" {
  type = string
  description = "VPC for default security group (used when security_group_ids is empty)"
  default = ""
}

variable "assign_public_ip" {
  type = bool
  default = false
}

variable "autoscaling_cpu_target" {
  type = number
  default = 70
}

variable "autoscaling_max_extra" {
  type = number
  default = 2
  description = "Max additional tasks above base replicas"
}

variable "log_retention_days" {
  type = number
  default = 7
}

