terraform {
  required_version = "< 1.18.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "6.72.0"
    }
  }

  backend "s3" {}
}

provider "aws" {
  region  = var.region
  profile = var.aws_profile

  default_tags {
    tags = merge(var.tags, {
      ManagedBy  = "provision-moatless-kubernetes"
      Deployment = var.deployment
    })
  }
}

