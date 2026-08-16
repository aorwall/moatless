output "cluster_name" {
  value = module.eks.cluster_name
}

output "cluster_primary_security_group_id" {
  value = module.eks.cluster_primary_security_group_id
}

output "backend_security_group_id" {
  value = aws_security_group.backend.id
}

output "backend_role_arn" {
  value = module.backend_irsa.arn
}

output "load_balancer_controller_role_arn" {
  value = module.load_balancer_controller_irsa.arn
}

output "certificate_arn" {
  value = var.dns_provider == "route53" ? aws_acm_certificate_validation.this[0].certificate_arn : aws_acm_certificate.this.arn
}

output "certificate_validation_records" {
  value = {
    for option in aws_acm_certificate.this.domain_validation_options : option.domain_name => {
      name  = option.resource_record_name
      type  = option.resource_record_type
      value = option.resource_record_value
    }
  }
}

output "workspace_bucket" {
  value = data.aws_s3_bucket.workspace.bucket
}

output "database_secret_arn" {
  value = aws_db_instance.this.master_user_secret[0].secret_arn
}

output "database_endpoint" {
  value = aws_db_instance.this.address
}

output "database_port" {
  value = aws_db_instance.this.port
}

output "database_name" {
  value = aws_db_instance.this.db_name
}

output "cache_endpoint" {
  value = aws_elasticache_replication_group.this.primary_endpoint_address
}

output "vpc_id" {
  value = module.vpc.vpc_id
}
