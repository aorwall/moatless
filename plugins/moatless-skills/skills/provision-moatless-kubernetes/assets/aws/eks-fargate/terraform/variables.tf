variable "deployment" {
  description = "Stable deployment name used as the resource prefix."
  type        = string
}

variable "region" {
  type = string
}

variable "aws_profile" {
  type = string
}

variable "cluster_version" {
  type = string
}

variable "availability_zones" {
  type = list(string)

  validation {
    condition     = length(var.availability_zones) == 2
    error_message = "Exactly two availability zones are required."
  }
}

variable "vpc_cidr" {
  type = string
}

variable "api_public_access_cidrs" {
  type = list(string)

  validation {
    condition     = length(var.api_public_access_cidrs) > 0 && !contains(var.api_public_access_cidrs, "0.0.0.0/0")
    error_message = "Provide at least one restricted API CIDR; 0.0.0.0/0 is rejected."
  }
}

variable "app_namespace" {
  type = string
}

variable "sandbox_namespace" {
  type = string
}

variable "app_domain" {
  type = string
}

variable "preview_domain" {
  type = string
}

variable "dns_provider" {
  description = "DNS owner: route53 manages records; external emits a DNS handoff."
  type        = string

  validation {
    condition     = contains(["route53", "external"], var.dns_provider)
    error_message = "dns_provider must be route53 or external."
  }
}

variable "route53_zone_id" {
  type    = string
  default = ""

  validation {
    condition     = var.dns_provider != "route53" || length(var.route53_zone_id) > 0
    error_message = "route53_zone_id is required when dns_provider is route53."
  }
}

variable "workspace_bucket" {
  type = string
}

variable "tags" {
  type    = map(string)
  default = {}
}
