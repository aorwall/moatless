locals {
  private_subnets     = [for index in range(2) : cidrsubnet(var.vpc_cidr, 4, index)]
  public_subnets      = [for index in range(2) : cidrsubnet(var.vpc_cidr, 8, index + 128)]
  database_subnets    = [for index in range(2) : cidrsubnet(var.vpc_cidr, 8, index + 160)]
  elasticache_subnets = [for index in range(2) : cidrsubnet(var.vpc_cidr, 8, index + 192)]
}

data "aws_caller_identity" "current" {}

data "aws_s3_bucket" "workspace" {
  bucket = var.workspace_bucket
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "6.6.1"

  name = var.deployment
  cidr = var.vpc_cidr
  azs  = var.availability_zones

  private_subnets     = local.private_subnets
  public_subnets      = local.public_subnets
  database_subnets    = local.database_subnets
  elasticache_subnets = local.elasticache_subnets

  enable_nat_gateway              = true
  single_nat_gateway              = true
  enable_dns_hostnames            = true
  enable_dns_support              = true
  create_database_subnet_group    = true
  create_elasticache_subnet_group = true

  public_subnet_tags = {
    "kubernetes.io/role/elb" = "1"
  }
  private_subnet_tags = {
    "kubernetes.io/role/internal-elb" = "1"
  }
}

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "21.24.2"

  name               = var.deployment
  kubernetes_version = var.cluster_version
  vpc_id             = module.vpc.vpc_id
  subnet_ids         = module.vpc.private_subnets

  authentication_mode                      = "API_AND_CONFIG_MAP"
  enable_cluster_creator_admin_permissions = true
  endpoint_private_access                  = true
  endpoint_public_access                   = true
  endpoint_public_access_cidrs             = var.api_public_access_cidrs
  enable_irsa                              = true

  addons = {
    coredns = {
      most_recent          = true
      configuration_values = jsonencode({ computeType = "Fargate" })
    }
    kube-proxy = {
      most_recent = true
    }
    vpc-cni = {
      before_compute = true
      most_recent    = true
    }
  }

  fargate_profiles = {
    system = {
      subnet_ids = module.vpc.private_subnets
      selectors = [{
        namespace = "kube-system"
      }]
    }
    application = {
      subnet_ids = module.vpc.private_subnets
      selectors = [{
        namespace = var.app_namespace
      }]
    }
    sandboxes = {
      subnet_ids = module.vpc.private_subnets
      selectors = [{
        namespace = var.sandbox_namespace
      }]
    }
  }
}

resource "aws_security_group" "backend" {
  name        = "${var.deployment}-backend-pods"
  description = "Security group selected only by Moatless backend pods"
  vpc_id      = module.vpc.vpc_id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_security_group" "database" {
  name        = "${var.deployment}-database"
  description = "PostgreSQL from Moatless backend pods"
  vpc_id      = module.vpc.vpc_id
}

resource "aws_vpc_security_group_ingress_rule" "database_from_backend" {
  security_group_id            = aws_security_group.database.id
  referenced_security_group_id = aws_security_group.backend.id
  from_port                    = 5432
  to_port                      = 5432
  ip_protocol                  = "tcp"
}

resource "aws_security_group" "cache" {
  name        = "${var.deployment}-cache"
  description = "Valkey from Moatless backend pods"
  vpc_id      = module.vpc.vpc_id
}

resource "aws_vpc_security_group_ingress_rule" "cache_from_backend" {
  security_group_id            = aws_security_group.cache.id
  referenced_security_group_id = aws_security_group.backend.id
  from_port                    = 6379
  to_port                      = 6379
  ip_protocol                  = "tcp"
}

resource "aws_db_instance" "this" {
  identifier                  = var.deployment
  engine                      = "postgres"
  engine_version              = "17"
  instance_class              = "db.t4g.micro"
  db_name                     = "moatless"
  username                    = "moatless"
  manage_master_user_password = true
  allocated_storage           = 20
  max_allocated_storage       = 100
  storage_type                = "gp3"
  storage_encrypted           = true
  multi_az                    = false
  publicly_accessible         = false
  db_subnet_group_name        = module.vpc.database_subnet_group_name
  vpc_security_group_ids      = [aws_security_group.database.id]
  backup_retention_period     = 1
  skip_final_snapshot         = false
  final_snapshot_identifier   = "${var.deployment}-final"
  deletion_protection         = false
  apply_immediately           = true
}

resource "aws_elasticache_replication_group" "this" {
  replication_group_id       = var.deployment
  description                = "Moatless Valkey cache"
  engine                     = "valkey"
  engine_version             = "8.2"
  node_type                  = "cache.t4g.micro"
  num_cache_clusters         = 1
  port                       = 6379
  transit_encryption_enabled = true
  at_rest_encryption_enabled = true
  automatic_failover_enabled = false
  subnet_group_name          = module.vpc.elasticache_subnet_group_name
  security_group_ids         = [aws_security_group.cache.id]
  apply_immediately          = true
}

resource "aws_acm_certificate" "this" {
  domain_name               = var.app_domain
  subject_alternative_names = ["*.${var.preview_domain}"]
  validation_method         = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "certificate_validation" {
  for_each = var.dns_provider == "route53" ? {
    for option in aws_acm_certificate.this.domain_validation_options : option.domain_name => {
      name   = option.resource_record_name
      record = option.resource_record_value
      type   = option.resource_record_type
    }
  } : {}

  zone_id = var.route53_zone_id
  name    = each.value.name
  type    = each.value.type
  records = [each.value.record]
  ttl     = 60
}

resource "aws_acm_certificate_validation" "this" {
  count = var.dns_provider == "route53" ? 1 : 0

  certificate_arn         = aws_acm_certificate.this.arn
  validation_record_fqdns = [for record in aws_route53_record.certificate_validation : record.fqdn]
}

module "backend_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts"
  version = "6.8.0"

  name            = "${var.deployment}-backend"
  use_name_prefix = false
  oidc_providers = {
    cluster = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["${var.app_namespace}:moatless-backend"]
    }
  }
  permissions = {
    WorkspaceList = {
      actions   = ["s3:ListBucket", "s3:GetBucketLocation"]
      resources = [data.aws_s3_bucket.workspace.arn]
    }
    WorkspaceObjects = {
      actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload"]
      resources = ["${data.aws_s3_bucket.workspace.arn}/*"]
    }
  }
}

module "load_balancer_controller_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts"
  version = "6.8.0"

  name                                   = "${var.deployment}-aws-load-balancer-controller"
  use_name_prefix                        = false
  attach_load_balancer_controller_policy = true
  oidc_providers = {
    cluster = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["kube-system:aws-load-balancer-controller"]
    }
  }
}
