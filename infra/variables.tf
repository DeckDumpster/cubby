variable "aws_region" {
  type    = string
  default = "us-west-2"
}

variable "deploy_role_arn" {
  type        = string
  description = "DeployRoleArn output of TerraformBootstrap (role terraform-deploy)."
}

variable "cloudflare_account_id" {
  type = string
}

variable "cloudflare_zone_id" {
  type        = string
  description = "Zone id of gantt.pub."
}

variable "hostname" {
  type    = string
  default = "cubby.gantt.pub"
}

variable "origin" {
  type    = string
  default = "http://127.0.0.1:8790"
}

variable "parent_email" {
  type    = string
  default = "ryan@gantt.pub"
}

variable "session_duration" {
  type    = string
  default = "24h"
}
