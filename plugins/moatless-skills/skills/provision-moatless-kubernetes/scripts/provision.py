#!/usr/bin/env python3
"""Plan, provision, verify, and retire a Moatless EKS Fargate environment."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.parse
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


SKILL_DIR = Path(__file__).resolve().parents[1]
ASSETS = SKILL_DIR / "assets"
AWS_FARGATE = ASSETS / "aws" / "eks-fargate"
METHOD_MARKER = ".moatless-provisioning-method"
LBC_CHART_VERSION = "3.5.0"
TRAEFIK_CHART_VERSION = "41.2.0"


class ProvisionError(RuntimeError):
    pass


def run(
    command: list[str],
    *,
    capture: bool = False,
    check: bool = True,
    cwd: Path | None = None,
    input_text: str | None = None,
) -> str:
    rendered = " ".join(command)
    print(f"+ {rendered}", file=sys.stderr)
    result = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        input=input_text,
        stdout=subprocess.PIPE if capture else None,
    )
    if check and result.returncode != 0:
        raise ProvisionError(f"command failed ({result.returncode}): {rendered}")
    return (result.stdout or "").strip()


def command_exists(name: str) -> bool:
    return shutil.which(name) is not None


def load_config(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            config = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ProvisionError(f"cannot read {path}: {error}") from error
    validate_config(config)
    return config


def validate_config(config: dict[str, Any]) -> None:
    expected = {
        "schema_version": 1,
        "provider": "aws",
        "compute_mode": "fargate",
    }
    for key, value in expected.items():
        if config.get(key) != value:
            raise ProvisionError(f"{key} must be {value!r}")
    if config.get("provisioning_method") not in {"terraform", "cli"}:
        raise ProvisionError("provisioning_method must be 'terraform' or 'cli'")
    if config.get("dns_provider") not in {"route53", "external"}:
        raise ProvisionError("dns_provider must be 'route53' or 'external'")
    if config["dns_provider"] == "external" and config["provisioning_method"] != "terraform":
        raise ProvisionError("external DNS is supported only by the terraform provisioning method")
    required = [
        "deployment",
        "region",
        "aws_profile",
        "cluster_version",
        "image_tag",
        "app_namespace",
        "sandbox_namespace",
        "app_domain",
        "preview_domain",
        "dns_zone",
        "vpc_cidr",
        "state_bucket",
        "workspace_bucket",
    ]
    for key in required:
        if not str(config.get(key, "")).strip():
            raise ProvisionError(f"{key} must not be empty")
    if not re.fullmatch(r"[a-z][a-z0-9-]{2,31}", config["deployment"]):
        raise ProvisionError("deployment must be 3-32 lowercase letters, digits, or hyphens")
    if config["cluster_version"] != "1.35":
        raise ProvisionError("this asset set is tested only with EKS 1.35")
    if config["dns_provider"] == "route53" and not str(config.get("route53_zone_id", "")).strip():
        raise ProvisionError("route53_zone_id is required when dns_provider is route53")
    zone_name = config["dns_zone"].rstrip(".")
    for key in ["app_domain", "preview_domain"]:
        domain = config[key].rstrip(".")
        if domain != zone_name and not domain.endswith(f".{zone_name}"):
            raise ProvisionError(f"{key}={domain} is outside DNS zone {zone_name}")
    if config["app_domain"].rstrip(".") == config["preview_domain"].rstrip("."):
        raise ProvisionError("app_domain and preview_domain must be different")
    zones = config.get("availability_zones", [])
    if len(zones) != 2 or zones[0] == zones[1]:
        raise ProvisionError("availability_zones must contain exactly two distinct zones")
    try:
        network = ipaddress.ip_network(config["vpc_cidr"])
    except ValueError as error:
        raise ProvisionError(f"vpc_cidr is invalid: {error}") from error
    if network.version != 4 or network.prefixlen > 20:
        raise ProvisionError("vpc_cidr must be an IPv4 network at least as large as /20")
    cidrs = config.get("api_public_access_cidrs", [])
    if not cidrs:
        raise ProvisionError("api_public_access_cidrs must contain at least one restricted CIDR")
    for cidr in cidrs:
        try:
            parsed = ipaddress.ip_network(cidr)
        except ValueError as error:
            raise ProvisionError(f"invalid API access CIDR {cidr!r}: {error}") from error
        if str(parsed) in {"0.0.0.0/0", "::/0"}:
            raise ProvisionError("public EKS API access cannot be open to the internet")
    for key in ["state_bucket", "workspace_bucket"]:
        if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", config[key]):
            raise ProvisionError(f"{key} is not a valid S3 bucket name")
    if config["state_bucket"] == config["workspace_bucket"]:
        raise ProvisionError("state_bucket and workspace_bucket must be different")


def canonical_hash(config: dict[str, Any]) -> str:
    payload = json.dumps(config, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def aws(config: dict[str, Any], *arguments: str, capture: bool = False) -> str:
    return run(
        [
            "aws",
            *arguments,
            "--profile",
            config["aws_profile"],
            "--region",
            config["region"],
        ],
        capture=capture,
    )


def aws_json(config: dict[str, Any], *arguments: str) -> Any:
    output = aws(config, *arguments, "--output", "json", capture=True)
    return json.loads(output)


def identity(config: dict[str, Any]) -> dict[str, Any]:
    return aws_json(config, "sts", "get-caller-identity")


def check_approval(actual: str | None, expected: str) -> None:
    if actual != expected:
        raise ProvisionError(f"refusing mutation: pass --approve {expected}")


def write_json(path: Path, value: Any, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    path.chmod(mode)


def write_private_text(path: Path, content: str) -> None:
    if path.is_symlink():
        raise ProvisionError(f"refusing to write secret values through symlink {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            descriptor = -1
            handle.write(content)
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def work_dir(args: argparse.Namespace, config: dict[str, Any]) -> Path:
    destination = Path(
        args.work_dir or f".moatless-infra/{config['deployment']}"
    ).resolve()
    unsafe = {
        Path(destination.anchor),
        Path.home().resolve(),
        Path.cwd().resolve(),
    }
    if destination in unsafe:
        raise ProvisionError(
            f"work directory must be a dedicated child directory, not {destination}"
        )
    return destination


def cmd_init(args: argparse.Namespace) -> None:
    output = Path(args.output)
    if output.exists() and not args.force:
        raise ProvisionError(f"{output} exists; pass --force to replace it")
    probe = {
        "aws_profile": args.aws_profile,
        "region": args.region,
    }
    caller = identity(probe)
    zones = aws_json(
        probe,
        "ec2",
        "describe-availability-zones",
        "--filters",
        "Name=state,Values=available",
    )["AvailabilityZones"]
    zone_names = [zone["ZoneName"] for zone in zones[:2]]
    if len(zone_names) != 2:
        raise ProvisionError(f"region {args.region} did not return two available zones")
    account = caller["Account"]
    deployment = args.deployment
    content = f'''schema_version = 1
deployment = "{deployment}"
provider = "aws"
compute_mode = "fargate"
provisioning_method = "{args.method}"
region = "{args.region}"
aws_profile = "{args.aws_profile}"
cluster_version = "1.35"
image_tag = "{args.image_tag}"

app_namespace = "moatless"
sandbox_namespace = "moatless-sandboxes"
app_domain = "{args.app_domain}"
preview_domain = "{args.preview_domain}"
dns_provider = "{args.dns_provider}"
dns_zone = "{args.dns_zone}"
route53_zone_id = "{args.route53_zone_id or ''}"

vpc_cidr = "10.42.0.0/16"
api_public_access_cidrs = ["{args.api_cidr}"]
availability_zones = ["{zone_names[0]}", "{zone_names[1]}"]

state_bucket = "{deployment}-tfstate-{account}-{args.region}"
workspace_bucket = "{deployment}-workspace-{account}-{args.region}"

[tags]
Environment = "dev"
'''
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(content)
    print(f"Wrote {output} for AWS account {account} ({caller['Arn']}).")


def parse_version(name: str, output: str) -> tuple[int, int, int]:
    match = re.search(r"v?(\d+)\.(\d+)(?:\.(\d+))?", output)
    if not match:
        raise ProvisionError(f"could not parse {name} version from: {output}")
    return tuple(int(part or 0) for part in match.groups())


def require_version(
    name: str,
    minimum: tuple[int, ...],
    *,
    major: int | None = None,
    maximum_exclusive: tuple[int, ...] | None = None,
) -> str:
    if not command_exists(name):
        raise ProvisionError(f"required command not found: {name}")
    flag = "version" if name in {"kubectl", "helm", "terraform", "eksctl"} else "--version"
    arguments = [name, flag]
    if name == "kubectl":
        arguments.append("--client")
    output = run(arguments, capture=True)
    version = parse_version(name, output)
    if major is not None and version[0] != major:
        raise ProvisionError(f"{name} {major}.x is required; found {output}")
    if version < minimum:
        raise ProvisionError(f"{name} >= {'.'.join(map(str, minimum))} is required; found {output}")
    if maximum_exclusive is not None and version >= maximum_exclusive:
        raise ProvisionError(
            f"{name} < {'.'.join(map(str, maximum_exclusive))} is required; found {output}"
        )
    return output.splitlines()[0]


def require_helm_version() -> str:
    if not command_exists("helm"):
        raise ProvisionError("required command not found: helm")
    output = run(["helm", "version"], capture=True)
    version = parse_version("helm", output)
    supported = (version[0] == 3 and version >= (3, 20, 0)) or (
        version[0] == 4 and version >= (4, 1, 0)
    )
    if not supported:
        raise ProvisionError(
            "EKS 1.35 requires Helm 3.20+ or Helm 4.1+; "
            f"found {output}"
        )
    return output.splitlines()[0]


def cmd_preflight(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    versions = {
        "aws": require_version("aws", (2, 0)),
        "kubectl": require_version("kubectl", (1, 31)),
        "helm": require_helm_version(),
    }
    if config["provisioning_method"] == "terraform":
        versions["terraform"] = require_version(
            "terraform", (1, 14), maximum_exclusive=(1, 16)
        )
    else:
        versions["eksctl"] = require_version("eksctl", (0, 229))
    caller = identity(config)
    zones = aws_json(config, "ec2", "describe-availability-zones")["AvailabilityZones"]
    available = {zone["ZoneName"] for zone in zones if zone["State"] == "available"}
    missing = set(config["availability_zones"]) - available
    if missing:
        raise ProvisionError(f"unavailable availability zones: {sorted(missing)}")
    zone_name = config["dns_zone"].rstrip(".")
    if config["dns_provider"] == "route53":
        hosted_zone = aws_json(
            config,
            "route53",
            "get-hosted-zone",
            "--id",
            config["route53_zone_id"],
        )["HostedZone"]
        actual_zone = hosted_zone["Name"].rstrip(".")
        if actual_zone != zone_name:
            raise ProvisionError(
                f"route53_zone_id belongs to {actual_zone}, expected configured DNS zone {zone_name}"
            )
    report = {
        "ready": True,
        "identity": caller,
        "region": config["region"],
        "availabilityZones": config["availability_zones"],
        "dnsProvider": config["dns_provider"],
        "dnsZone": zone_name,
        "appDomain": config["app_domain"],
        "previewDomain": config["preview_domain"],
        "method": config["provisioning_method"],
        "versions": versions,
        "costDrivers": [
            "EKS control plane",
            "one NAT gateway plus data processing",
            "Fargate pod CPU and memory",
            "RDS db.t4g.micro PostgreSQL 17",
            "ElastiCache cache.t4g.micro Valkey 8.2",
            "ALB, S3, logs, and data transfer",
        ],
        "securityNote": "Fargate does not enforce EKS VPC CNI NetworkPolicy; run trusted repositories only.",
    }
    print(json.dumps(report, indent=2))


def plan_document(config: dict[str, Any], action: str) -> dict[str, Any]:
    caller = identity(config)
    return {
        "schemaVersion": 1,
        "action": action,
        "deployment": config["deployment"],
        "configSha256": canonical_hash(config),
        "account": caller["Account"],
        "principalArn": caller["Arn"],
        "region": config["region"],
        "method": config["provisioning_method"],
        "createdAt": datetime.now(UTC).isoformat(),
    }


def cmd_bootstrap_plan(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    document = plan_document(config, "bootstrap")
    document["buckets"] = [
        {"name": config["state_bucket"], "purpose": "Terraform state and ownership marker", "retained": True},
        {"name": config["workspace_bucket"], "purpose": "Moatless workspace and uploads", "retained": True},
    ]
    write_json(Path(args.output), document)
    print(json.dumps(document, indent=2))


def validate_plan(config: dict[str, Any], path: Path, action: str) -> dict[str, Any]:
    try:
        document = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ProvisionError(f"cannot read plan {path}: {error}") from error
    if document.get("action") != action or document.get("deployment") != config["deployment"]:
        raise ProvisionError(f"{path} does not describe {action} for {config['deployment']}")
    if document.get("configSha256") != canonical_hash(config):
        raise ProvisionError("config changed after the plan was created; create a new plan")
    caller = identity(config)
    if document.get("account") != caller["Account"] or document.get("principalArn") != caller["Arn"]:
        raise ProvisionError("active AWS identity differs from the identity that created the plan")
    return document


def bucket_exists(config: dict[str, Any], bucket: str) -> bool:
    result = subprocess.run(
        ["aws", "s3api", "head-bucket", "--bucket", bucket, "--profile", config["aws_profile"]],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def configure_bucket(config: dict[str, Any], bucket: str, purpose: str) -> None:
    if not bucket_exists(config, bucket):
        arguments = ["s3api", "create-bucket", "--bucket", bucket]
        if config["region"] != "us-east-1":
            arguments += ["--create-bucket-configuration", f"LocationConstraint={config['region']}"]
        aws(config, *arguments)
    aws(config, "s3api", "put-bucket-versioning", "--bucket", bucket, "--versioning-configuration", "Status=Enabled")
    aws(config, "s3api", "put-bucket-encryption", "--bucket", bucket, "--server-side-encryption-configuration", '{"Rules":[{"ApplyServerSideEncryptionByDefault":{"SSEAlgorithm":"AES256"},"BucketKeyEnabled":true}]}' )
    aws(config, "s3api", "put-public-access-block", "--bucket", bucket, "--public-access-block-configuration", "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true")
    tags = [{"Key": "ManagedBy", "Value": "provision-moatless-kubernetes"}, {"Key": "Deployment", "Value": config["deployment"]}, {"Key": "Purpose", "Value": purpose}]
    tags.extend({"Key": key, "Value": str(value)} for key, value in config.get("tags", {}).items())
    aws(config, "s3api", "put-bucket-tagging", "--bucket", bucket, "--tagging", json.dumps({"TagSet": tags}))


def put_method_marker(config: dict[str, Any]) -> None:
    marker = {
        "schemaVersion": 1,
        "deployment": config["deployment"],
        "method": config["provisioning_method"],
        "account": identity(config)["Account"],
        "region": config["region"],
    }
    with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
        json.dump(marker, handle)
        handle.flush()
        aws(config, "s3api", "put-object", "--bucket", config["state_bucket"], "--key", f"ownership/{config['deployment']}.json", "--body", handle.name)


def read_method_marker(config: dict[str, Any]) -> dict[str, Any] | None:
    with tempfile.TemporaryDirectory() as temporary:
        output = Path(temporary) / "ownership.json"
        result = subprocess.run(
            [
                "aws", "s3api", "get-object", "--bucket", config["state_bucket"],
                "--key", f"ownership/{config['deployment']}.json", str(output),
                "--profile", config["aws_profile"], "--region", config["region"],
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if result.returncode != 0:
            return None
        try:
            return json.loads(output.read_text())
        except (OSError, json.JSONDecodeError) as error:
            raise ProvisionError("state bucket ownership marker is invalid") from error


def validate_remote_ownership(config: dict[str, Any]) -> None:
    marker = read_method_marker(config)
    expected = {
        "deployment": config["deployment"],
        "method": config["provisioning_method"],
        "account": identity(config)["Account"],
        "region": config["region"],
    }
    if marker is None:
        raise ProvisionError("state bucket exists without this deployment's ownership marker")
    for key, value in expected.items():
        if marker.get(key) != value:
            raise ProvisionError(f"state bucket ownership mismatch for {key}")


def validate_bucket_tags(config: dict[str, Any], bucket: str) -> None:
    try:
        tagging = aws_json(config, "s3api", "get-bucket-tagging", "--bucket", bucket)
    except ProvisionError as error:
        raise ProvisionError(f"existing bucket s3://{bucket} has no readable ownership tags") from error
    tags = {item["Key"]: item["Value"] for item in tagging.get("TagSet", [])}
    if tags.get("ManagedBy") != "provision-moatless-kubernetes" or tags.get("Deployment") != config["deployment"]:
        raise ProvisionError(f"existing bucket s3://{bucket} is not owned by this deployment")


def cmd_bootstrap(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    check_approval(args.approve, f"BOOTSTRAP-{config['deployment']}")
    validate_plan(config, Path(args.plan), "bootstrap")
    state_exists = bucket_exists(config, config["state_bucket"])
    if state_exists:
        validate_bucket_tags(config, config["state_bucket"])
        marker = read_method_marker(config)
        if marker is not None:
            validate_remote_ownership(config)
    workspace_exists = bucket_exists(config, config["workspace_bucket"])
    if workspace_exists:
        validate_bucket_tags(config, config["workspace_bucket"])
    configure_bucket(config, config["state_bucket"], "state")
    put_method_marker(config)
    configure_bucket(config, config["workspace_bucket"], "workspace")
    print("Bootstrap complete. Both versioned buckets are retained by destroy.")


def subnet_cidrs(vpc_cidr: str) -> dict[str, list[str]]:
    network = ipaddress.ip_network(vpc_cidr)
    private = list(network.subnets(new_prefix=network.prefixlen + 4))[:2]
    small = list(network.subnets(new_prefix=network.prefixlen + 8))
    return {
        "private": [str(item) for item in private],
        "public": [str(small[index]) for index in (128, 129)],
        "database": [str(small[index]) for index in (160, 161)],
        "cache": [str(small[index]) for index in (192, 193)],
    }


def copy_assets(source: Path, destination: Path) -> None:
    shutil.copytree(source, destination, dirs_exist_ok=True)


def directory_hash(roots: list[Path], relative_to: Path) -> str:
    digest = hashlib.sha256()
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            digest.update(str(path.relative_to(relative_to)).encode())
            digest.update(b"\0")
            digest.update(path.read_bytes())
            digest.update(b"\0")
    return digest.hexdigest()


def source_asset_hash(method: str) -> str:
    return directory_hash(
        [AWS_FARGATE / method, AWS_FARGATE / "kubernetes"],
        AWS_FARGATE,
    )


def cmd_render(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    destination = work_dir(args, config)
    marker = destination / METHOD_MARKER
    generated_paths = [destination / config["provisioning_method"], destination / "kubernetes"]
    if (
        not marker.exists()
        and any(path.exists() for path in generated_paths)
    ):
        raise ProvisionError(
            "work directory contains unowned generated paths; choose an empty "
            "directory or one with a matching provisioning marker"
        )
    destination.mkdir(parents=True, exist_ok=True)
    if marker.exists() and marker.read_text().strip() != config["provisioning_method"]:
        raise ProvisionError("work directory belongs to another provisioning method")
    marker.write_text(config["provisioning_method"] + "\n")
    method = config["provisioning_method"]
    for generated in generated_paths:
        if generated.exists():
            shutil.rmtree(generated)
    copy_assets(AWS_FARGATE / method, destination / method)
    copy_assets(AWS_FARGATE / "kubernetes", destination / "kubernetes")
    if method == "terraform":
        tfvars = {
            key: config[key]
            for key in [
                "deployment", "region", "aws_profile", "cluster_version", "availability_zones",
                "vpc_cidr", "api_public_access_cidrs", "app_namespace", "sandbox_namespace",
                "app_domain", "preview_domain", "dns_provider", "route53_zone_id",
                "workspace_bucket", "tags",
            ]
        }
        write_json(destination / "terraform" / "terraform.auto.tfvars.json", tfvars)
        backend = f'''bucket  = "{config['state_bucket']}"
key     = "deployments/{config['deployment']}/terraform.tfstate"
region  = "{config['region']}"
profile = "{config['aws_profile']}"
encrypt = true
use_lockfile = true
'''
        (destination / "terraform" / "backend.hcl").write_text(backend)
    else:
        cidrs = subnet_cidrs(config["vpc_cidr"])
        parameters = {
            "Deployment": config["deployment"],
            "AvailabilityZone1": config["availability_zones"][0],
            "AvailabilityZone2": config["availability_zones"][1],
            "VpcCidr": config["vpc_cidr"],
            "PrivateSubnet1Cidr": cidrs["private"][0],
            "PrivateSubnet2Cidr": cidrs["private"][1],
            "PublicSubnet1Cidr": cidrs["public"][0],
            "PublicSubnet2Cidr": cidrs["public"][1],
            "DatabaseSubnet1Cidr": cidrs["database"][0],
            "DatabaseSubnet2Cidr": cidrs["database"][1],
            "CacheSubnet1Cidr": cidrs["cache"][0],
            "CacheSubnet2Cidr": cidrs["cache"][1],
            "HostedZoneId": config["route53_zone_id"],
            "AppDomain": config["app_domain"],
            "PreviewDomain": config["preview_domain"],
            "WorkspaceBucket": config["workspace_bucket"],
        }
        write_json(destination / "cli" / "foundation-parameters.json", parameters)
    write_json(
        destination / "render.json",
        {
            "assetSha256": source_asset_hash(method),
            "configSha256": canonical_hash(config),
            "method": method,
        },
    )
    print(f"Rendered {method} assets to {destination}")


def ensure_render_matches(config: dict[str, Any], destination: Path) -> None:
    try:
        render = json.loads((destination / "render.json").read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ProvisionError("rendered assets missing; run render first") from error
    expected = {
        "assetSha256": source_asset_hash(config["provisioning_method"]),
        "configSha256": canonical_hash(config),
        "method": config["provisioning_method"],
    }
    if render != expected:
        raise ProvisionError("rendered assets are stale or use a different method")


def deployment_artifact_hash(config: dict[str, Any], destination: Path) -> str:
    digest = hashlib.sha256()
    roots = [destination / config["provisioning_method"], destination / "kubernetes"]
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or ".terraform" in path.parts or path.name == "eksctl.yaml":
                continue
            digest.update(str(path.relative_to(destination)).encode())
            digest.update(b"\0")
            digest.update(path.read_bytes())
            digest.update(b"\0")
    return digest.hexdigest()


def cmd_plan(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    destination = work_dir(args, config)
    ensure_render_matches(config, destination)
    validate_remote_ownership(config)
    document = plan_document(config, "apply")
    document.update(
        {
            "appDomain": config["app_domain"],
            "previewDomain": config["preview_domain"],
            "dnsProvider": config["dns_provider"],
            "costApprovalRequired": True,
        }
    )
    if config["provisioning_method"] == "terraform":
        directory = destination / "terraform"
        run(["terraform", "fmt", "-check", "-recursive"], cwd=directory)
        run(["terraform", "init", "-reconfigure", "-backend-config=backend.hcl"], cwd=directory)
        run(["terraform", "validate"], cwd=directory)
        run(["terraform", "plan", "-out=apply.tfplan"], cwd=directory)
        document["artifact"] = str(directory / "apply.tfplan")
    else:
        aws(config, "cloudformation", "validate-template", "--template-body", f"file://{destination / 'cli' / 'foundation.yaml'}")
        run(["eksctl", "version"], capture=True)
        document["artifact"] = str(destination / "cli" / "foundation.yaml")
        document["experimental"] = True
        document["planLimitation"] = (
            "The CLI path validates rendered CloudFormation and eksctl inputs but does not "
            "create an executable CloudFormation change set. Review the rendered assets before approval."
        )
    document["artifactSha256"] = deployment_artifact_hash(config, destination)
    write_json(destination / "apply-plan.json", document)
    print(json.dumps(document, indent=2))


def stack_outputs(config: dict[str, Any]) -> dict[str, str]:
    stacks = aws_json(config, "cloudformation", "describe-stacks", "--stack-name", f"{config['deployment']}-foundation")
    return {item["OutputKey"]: item["OutputValue"] for item in stacks["Stacks"][0]["Outputs"]}


def describe_cluster_optional(config: dict[str, Any]) -> dict[str, Any] | None:
    command = [
        "aws", "eks", "describe-cluster", "--name", config["deployment"],
        "--profile", config["aws_profile"], "--region", config["region"], "--output", "json",
    ]
    print(f"+ {' '.join(command)}", file=sys.stderr)
    result = subprocess.run(command, check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode == 0:
        return json.loads(result.stdout)["cluster"]
    if "ResourceNotFoundException" in result.stderr:
        return None
    raise ProvisionError(f"cannot inspect EKS cluster: {result.stderr.strip()}")


def render_eksctl(config: dict[str, Any], directory: Path, outputs: dict[str, str]) -> Path:
    replacements = {
        "__CLUSTER_NAME__": config["deployment"],
        "__REGION__": config["region"],
        "__CLUSTER_VERSION__": config["cluster_version"],
        "__VPC_ID__": outputs["VpcId"],
        "__API_CIDRS__": json.dumps(config["api_public_access_cidrs"]),
        "__AZ1__": config["availability_zones"][0],
        "__AZ2__": config["availability_zones"][1],
        "__PUBLIC_SUBNET_1__": outputs["PublicSubnet1"],
        "__PUBLIC_SUBNET_2__": outputs["PublicSubnet2"],
        "__PRIVATE_SUBNET_1__": outputs["PrivateSubnet1"],
        "__PRIVATE_SUBNET_2__": outputs["PrivateSubnet2"],
        "__APP_NAMESPACE__": config["app_namespace"],
        "__SANDBOX_NAMESPACE__": config["sandbox_namespace"],
    }
    content = (directory / "eksctl.yaml.tmpl").read_text()
    for key, value in replacements.items():
        content = content.replace(key, value)
    output = directory / "eksctl.yaml"
    output.write_text(content)
    return output


def terraform_outputs(directory: Path) -> dict[str, Any]:
    values = json.loads(run(["terraform", "output", "-json"], cwd=directory, capture=True))
    return {key: value["value"] for key, value in values.items()}


def update_kubeconfig(config: dict[str, Any]) -> str:
    context = f"{config['deployment']}-{config['region']}"
    previous = run(
        ["kubectl", "config", "current-context"],
        capture=True,
        check=False,
    )
    try:
        aws(
            config,
            "eks",
            "update-kubeconfig",
            "--name",
            config["deployment"],
            "--alias",
            context,
        )
    finally:
        if previous:
            run(["kubectl", "config", "use-context", previous])
    contexts = run(
        ["kubectl", "config", "get-contexts", "-o", "name"],
        capture=True,
    ).splitlines()
    if context not in contexts:
        raise ProvisionError(f"kubeconfig does not contain expected context {context}")
    return context


def ensure_namespaces(config: dict[str, Any], context: str) -> None:
    for namespace in [config["app_namespace"], config["sandbox_namespace"]]:
        manifest = run(
            [
                "kubectl", "--context", context, "create", "namespace", namespace,
                "--dry-run=client", "-o", "yaml",
            ],
            capture=True,
        )
        run(["kubectl", "--context", context, "apply", "-f", "-"], input_text=manifest)


def ensure_cli_roles(config: dict[str, Any], directory: Path, outputs: dict[str, Any]) -> None:
    account = identity(config)["Account"]
    policy_name = f"{config['deployment']}-aws-load-balancer-controller"
    lbc_policy_arn = f"arn:aws:iam::{account}:policy/{policy_name}"
    get_policy = subprocess.run(
        ["aws", "iam", "get-policy", "--policy-arn", lbc_policy_arn, "--profile", config["aws_profile"]],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if get_policy.returncode != 0:
        aws(config, "iam", "create-policy", "--policy-name", policy_name, "--policy-document", f"file://{directory / 'aws-load-balancer-controller-policy.json'}")
    roles = [
        (config["app_namespace"], "moatless-backend", f"{config['deployment']}-backend", outputs["WorkspacePolicyArn"]),
        ("kube-system", "aws-load-balancer-controller", f"{config['deployment']}-aws-load-balancer-controller", lbc_policy_arn),
    ]
    for namespace, service_account, role, policy in roles:
        run([
            "eksctl", "create", "iamserviceaccount", "--cluster", config["deployment"],
            "--region", config["region"], "--namespace", namespace, "--name", service_account,
            "--role-name", role, "--attach-policy-arn", policy, "--approve",
            "--role-only", "--profile", config["aws_profile"],
        ])
    outputs["backend_role_arn"] = f"arn:aws:iam::{account}:role/{config['deployment']}-backend"
    outputs["load_balancer_controller_role_arn"] = f"arn:aws:iam::{account}:role/{config['deployment']}-aws-load-balancer-controller"


def apply_template(path: Path, replacements: dict[str, str], output: Path) -> Path:
    content = path.read_text()
    for key, value in replacements.items():
        content = content.replace(key, value)
    output.write_text(content)
    return output


def install_cluster_components(config: dict[str, Any], destination: Path, outputs: dict[str, Any]) -> None:
    context = update_kubeconfig(config)
    ensure_namespaces(config, context)
    service_account = run(["kubectl", "--context", context, "-n", "kube-system", "create", "serviceaccount", "aws-load-balancer-controller", "--dry-run=client", "-o", "yaml"], capture=True)
    run(["kubectl", "--context", context, "apply", "-f", "-"], input_text=service_account)
    run(["kubectl", "--context", context, "-n", "kube-system", "annotate", "serviceaccount", "aws-load-balancer-controller", f"eks.amazonaws.com/role-arn={outputs['load_balancer_controller_role_arn']}", "--overwrite"])
    cluster_sg = outputs["cluster_primary_security_group_id"]
    backend_sg = outputs["backend_security_group_id"]
    policy = apply_template(
        destination / "kubernetes" / "backend-security-group-policy.yaml.tmpl",
        {"__APP_NAMESPACE__": config["app_namespace"], "__CLUSTER_SECURITY_GROUP_ID__": cluster_sg, "__BACKEND_SECURITY_GROUP_ID__": backend_sg},
        destination / "kubernetes" / "backend-security-group-policy.yaml",
    )
    run(["kubectl", "--context", context, "apply", "-f", str(policy)])
    lbc_values = apply_template(
        destination / "kubernetes" / "aws-lbc-values.yaml.tmpl",
        {"__CLUSTER_NAME__": config["deployment"], "__REGION__": config["region"], "__VPC_ID__": outputs["vpc_id"]},
        destination / "kubernetes" / "aws-lbc-values.yaml",
    )
    run(["helm", "repo", "add", "eks", "https://aws.github.io/eks-charts", "--force-update"])
    run(["helm", "repo", "add", "traefik", "https://traefik.github.io/charts", "--force-update"])
    run(["helm", "repo", "update"])
    run(["helm", "upgrade", "--install", "aws-load-balancer-controller", "eks/aws-load-balancer-controller", "--version", LBC_CHART_VERSION, "--namespace", "kube-system", "--values", str(lbc_values), "--kube-context", context, "--wait", "--timeout", "15m"])
    run(["helm", "upgrade", "--install", "traefik", "traefik/traefik", "--version", TRAEFIK_CHART_VERSION, "--namespace", config["app_namespace"], "--values", str(destination / "kubernetes" / "traefik-values.yaml"), "--kube-context", context, "--wait", "--timeout", "15m"])


def cmd_apply(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    check_approval(args.approve, f"APPLY-{config['deployment']}")
    destination = work_dir(args, config)
    document = validate_plan(config, destination / "apply-plan.json", "apply")
    ensure_render_matches(config, destination)
    validate_remote_ownership(config)
    if document.get("artifactSha256") != deployment_artifact_hash(config, destination):
        raise ProvisionError("rendered infrastructure changed after plan; create a new plan")
    if config["provisioning_method"] == "terraform":
        directory = destination / "terraform"
        run(["terraform", "apply", "apply.tfplan"], cwd=directory)
        outputs = terraform_outputs(directory)
    else:
        directory = destination / "cli"
        parameters = json.loads((directory / "foundation-parameters.json").read_text())
        parameter_args = [f"{key}={value}" for key, value in parameters.items()]
        aws(config, "cloudformation", "deploy", "--stack-name", f"{config['deployment']}-foundation", "--template-file", str(directory / "foundation.yaml"), "--capabilities", "CAPABILITY_NAMED_IAM", "--parameter-overrides", *parameter_args, "--tags", "ManagedBy=provision-moatless-kubernetes", f"Deployment={config['deployment']}")
        foundation = stack_outputs(config)
        eksctl_config = render_eksctl(config, directory, foundation)
        cluster = describe_cluster_optional(config)
        if cluster is None:
            run(["eksctl", "create", "cluster", "--config-file", str(eksctl_config), "--profile", config["aws_profile"]])
            cluster = describe_cluster_optional(config)
        if cluster is None:
            raise ProvisionError("EKS cluster was not found after eksctl completed")
        if cluster["version"] != config["cluster_version"] or cluster["resourcesVpcConfig"]["vpcId"] != foundation["VpcId"]:
            raise ProvisionError("existing EKS cluster does not match the configured version and VPC")
        if cluster["status"] != "ACTIVE":
            aws(config, "eks", "wait", "cluster-active", "--name", config["deployment"])
            cluster = describe_cluster_optional(config)
            if cluster is None:
                raise ProvisionError("EKS cluster disappeared while waiting for ACTIVE")
        profiles = set(
            aws_json(config, "eks", "list-fargate-profiles", "--cluster-name", config["deployment"])[
                "fargateProfileNames"
            ]
        )
        missing_profiles = {"system", "application", "sandboxes"} - profiles
        if missing_profiles:
            raise ProvisionError(
                f"existing EKS cluster is missing Fargate profiles: {sorted(missing_profiles)}"
            )
        context = update_kubeconfig(config)
        ensure_namespaces(config, context)
        run(["kubectl", "--context", context, "-n", "kube-system", "patch", "deployment", "coredns", "--type", "json", "-p", '[{"op":"remove","path":"/spec/template/metadata/annotations/eks.amazonaws.com~1compute-type"}]'], check=False)
        outputs = {
            "cluster_name": config["deployment"],
            "cluster_primary_security_group_id": cluster["resourcesVpcConfig"]["clusterSecurityGroupId"],
            "backend_security_group_id": foundation["BackendSecurityGroupId"],
            "certificate_arn": foundation["CertificateArn"],
            "workspace_bucket": config["workspace_bucket"],
            "database_secret_arn": foundation["DatabaseSecretArn"],
            "database_endpoint": foundation["DatabaseEndpoint"],
            "database_port": int(foundation["DatabasePort"]),
            "database_name": foundation["DatabaseName"],
            "cache_endpoint": foundation["CacheEndpoint"],
            "vpc_id": foundation["VpcId"],
        }
        ensure_cli_roles(config, directory, {**foundation, **outputs})
        account = identity(config)["Account"]
        outputs["backend_role_arn"] = f"arn:aws:iam::{account}:role/{config['deployment']}-backend"
        outputs["load_balancer_controller_role_arn"] = f"arn:aws:iam::{account}:role/{config['deployment']}-aws-load-balancer-controller"
    write_json(destination / "infrastructure.json", outputs)
    if config["dns_provider"] == "external":
        validation_handoff = {
            "schemaVersion": 1,
            "provider": "external",
            "zone": config["dns_zone"],
            "purpose": "ACM certificate validation",
            "records": list(outputs["certificate_validation_records"].values()),
        }
        validation_path = destination / "dns-validation-records.json"
        write_json(validation_path, validation_handoff)
        print(
            f"External DNS owns {config['dns_zone']}. Add the CNAME records in "
            f"{validation_path} before verify."
        )
    install_cluster_components(config, destination, outputs)
    print("Infrastructure and cluster components are ready. Run verify next.")


def read_outputs(config: dict[str, Any], destination: Path) -> dict[str, Any]:
    try:
        return json.loads((destination / "infrastructure.json").read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ProvisionError("infrastructure outputs missing; complete apply first") from error


def cmd_verify(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    destination = work_dir(args, config)
    outputs = read_outputs(config, destination)
    certificate = aws_json(
        config,
        "acm",
        "describe-certificate",
        "--certificate-arn",
        outputs["certificate_arn"],
    )["Certificate"]
    if certificate["Status"] != "ISSUED":
        hint = ""
        if config["dns_provider"] == "external":
            hint = f"; add the CNAME records from {destination / 'dns-validation-records.json'}"
        raise ProvisionError(f"ACM certificate status is {certificate['Status']}{hint}")
    context = update_kubeconfig(config)
    run(["kubectl", "--context", context, "wait", "--for=condition=Available", "deployment/coredns", "-n", "kube-system", "--timeout=10m"])
    run(["kubectl", "--context", context, "wait", "--for=condition=Available", "deployment/aws-load-balancer-controller", "-n", "kube-system", "--timeout=10m"])
    run(["kubectl", "--context", context, "wait", "--for=condition=Available", "deployment/traefik", "-n", config["app_namespace"], "--timeout=10m"])
    handoff = {
        "schemaVersion": 1,
        "provider": "aws",
        "computeMode": "fargate",
        "context": context,
        "appNamespace": config["app_namespace"],
        "sandboxNamespace": config["sandbox_namespace"],
        "region": config["region"],
        "appDomain": config["app_domain"],
        "previewDomain": config["preview_domain"],
        "certificateArn": outputs["certificate_arn"],
        "workspaceBucket": outputs["workspace_bucket"],
        "backendRoleArn": outputs["backend_role_arn"],
        "databaseSecretArn": outputs["database_secret_arn"],
        "databaseEndpoint": outputs["database_endpoint"],
        "databaseName": outputs["database_name"],
        "databasePort": outputs["database_port"],
        "cacheEndpoint": outputs["cache_endpoint"],
        "provisioningMethod": config["provisioning_method"],
        "imageTag": config["image_tag"],
    }
    output = Path(args.output or destination / "infrastructure-handoff.json")
    write_json(output, handoff)
    print(f"Verified cluster and wrote {output}")


def cmd_secrets_env(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    destination = work_dir(args, config)
    outputs = read_outputs(config, destination)
    secret = aws_json(config, "secretsmanager", "get-secret-value", "--secret-id", outputs["database_secret_arn"])
    credentials = json.loads(secret["SecretString"])
    username = urllib.parse.quote(credentials["username"], safe="")
    password = urllib.parse.quote(credentials["password"], safe="")
    database = urllib.parse.quote(outputs["database_name"], safe="")
    lines = [
        f"DATABASE_URL=postgresql://{username}:{password}@{outputs['database_endpoint']}:{outputs['database_port']}/{database}?sslmode=require",
        f"REDIS_URL=rediss://{outputs['cache_endpoint']}:6379/0",
    ]
    output = Path(args.output)
    if output.exists() and not args.force:
        raise ProvisionError(f"{output} exists; pass --force to replace it")
    write_private_text(output, "\n".join(lines) + "\n")
    print(f"Wrote database and cache connection values to {output} with mode 0600; values were not printed.")


def cmd_finalize_dns(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    check_approval(args.approve, f"DNS-{config['deployment']}")
    context = update_kubeconfig(config)
    ingress = json.loads(run(["kubectl", "--context", context, "-n", config["app_namespace"], "get", "ingress", "-o", "json"], capture=True))
    hostnames = {
        entry["hostname"]
        for item in ingress["items"]
        for entry in item.get("status", {}).get("loadBalancer", {}).get("ingress", [])
        if entry.get("hostname")
    }
    if len(hostnames) != 1:
        raise ProvisionError(f"expected one shared ALB hostname, found {sorted(hostnames)}")
    hostname = hostnames.pop()
    load_balancers = aws_json(config, "elbv2", "describe-load-balancers")["LoadBalancers"]
    load_balancer = next((item for item in load_balancers if item["DNSName"] == hostname), None)
    if load_balancer is None:
        raise ProvisionError(f"could not resolve ALB metadata for {hostname}")
    if config["dns_provider"] == "external":
        handoff = {
            "schemaVersion": 1,
            "provider": "external",
            "zone": config["dns_zone"],
            "purpose": "Moatless application routing",
            "records": [
                {
                    "name": config["app_domain"],
                    "type": "CNAME",
                    "value": hostname,
                    "proxied": False,
                    "note": "Use provider apex CNAME flattening when this is the zone apex.",
                },
                {
                    "name": f"*.{config['preview_domain']}",
                    "type": "CNAME",
                    "value": hostname,
                    "proxied": False,
                },
            ],
        }
        output = work_dir(args, config) / "dns-handoff.json"
        write_json(output, handoff)
        print(f"External DNS is unchanged. Add the CNAME records in {output}.")
        return
    changes = [
        {
            "Action": "UPSERT",
            "ResourceRecordSet": {
                "Name": name,
                "Type": "A",
                "AliasTarget": {"DNSName": hostname, "HostedZoneId": load_balancer["CanonicalHostedZoneId"], "EvaluateTargetHealth": True},
            },
        }
        for name in [config["app_domain"], f"*.{config['preview_domain']}"]
    ]
    change_batch = json.dumps({"Comment": f"Moatless {config['deployment']}", "Changes": changes})
    aws(config, "route53", "change-resource-record-sets", "--hosted-zone-id", config["route53_zone_id"], "--change-batch", change_batch)
    print(f"Upserted application and wildcard preview aliases to {hostname}")


def cmd_destroy_plan(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    destination = work_dir(args, config)
    ensure_render_matches(config, destination)
    validate_remote_ownership(config)
    document = plan_document(config, "destroy")
    document.update({
        "willDelete": ["EKS cluster", "Fargate profiles", "VPC and NAT gateway", "RDS instance after final snapshot", "Valkey cache", "ACM certificate", "controller IAM roles"],
        "willRetain": [config["state_bucket"], config["workspace_bucket"], "RDS final snapshot"],
    })
    if config["provisioning_method"] == "terraform":
        run(["terraform", "plan", "-destroy", "-out=destroy.tfplan"], cwd=destination / "terraform")
        document["artifactSha256"] = deployment_artifact_hash(config, destination)
    write_json(destination / "destroy-plan.json", document)
    print(json.dumps(document, indent=2))


def cmd_destroy(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    check_approval(args.approve, f"DESTROY-{config['deployment']}")
    destination = work_dir(args, config)
    document = validate_plan(config, destination / "destroy-plan.json", "destroy")
    validate_remote_ownership(config)
    if config["provisioning_method"] == "terraform" and document.get("artifactSha256") != deployment_artifact_hash(config, destination):
        raise ProvisionError("rendered infrastructure changed after destroy plan; create a new plan")
    if command_exists("kubectl"):
        context = update_kubeconfig(config)
        run(["helm", "uninstall", "moatless-vibe", "--namespace", config["app_namespace"], "--kube-context", context], check=False)
        run(["kubectl", "--context", context, "-n", config["app_namespace"], "delete", "ingress", "--all", "--ignore-not-found"], check=False)
        run(["helm", "uninstall", "traefik", "--namespace", config["app_namespace"], "--kube-context", context], check=False)
        run(["helm", "uninstall", "aws-load-balancer-controller", "--namespace", "kube-system", "--kube-context", context], check=False)
    if config["provisioning_method"] == "terraform":
        run(["terraform", "apply", "destroy.tfplan"], cwd=destination / "terraform")
    else:
        for namespace, name in [
            (config["app_namespace"], "moatless-backend"),
            ("kube-system", "aws-load-balancer-controller"),
        ]:
            run(
                [
                    "eksctl", "delete", "iamserviceaccount", "--cluster", config["deployment"],
                    "--region", config["region"], "--namespace", namespace, "--name", name,
                    "--approve", "--wait", "--profile", config["aws_profile"],
                ],
                check=False,
            )
        run(["eksctl", "delete", "cluster", "--name", config["deployment"], "--region", config["region"], "--profile", config["aws_profile"], "--wait"])
        aws(config, "cloudformation", "delete-stack", "--stack-name", f"{config['deployment']}-foundation")
        aws(config, "cloudformation", "wait", "stack-delete-complete", "--stack-name", f"{config['deployment']}-foundation")
        account = identity(config)["Account"]
        lbc_policy_arn = f"arn:aws:iam::{account}:policy/{config['deployment']}-aws-load-balancer-controller"
        aws(config, "iam", "delete-policy", "--policy-arn", lbc_policy_arn)
    print("Infrastructure destroyed. State/workspace buckets and the database snapshot were retained.")


def empty_versioned_bucket(config: dict[str, Any], bucket: str) -> None:
    key_marker: str | None = None
    version_marker: str | None = None
    while True:
        arguments = ["s3api", "list-object-versions", "--bucket", bucket]
        if key_marker:
            arguments += ["--key-marker", key_marker]
        if version_marker:
            arguments += ["--version-id-marker", version_marker]
        page = aws_json(config, *arguments)
        objects = [
            {"Key": item["Key"], "VersionId": item["VersionId"]}
            for collection in [page.get("Versions", []), page.get("DeleteMarkers", [])]
            for item in collection
        ]
        for start in range(0, len(objects), 1000):
            aws(config, "s3api", "delete-objects", "--bucket", bucket, "--delete", json.dumps({"Objects": objects[start : start + 1000], "Quiet": True}))
        if not page.get("IsTruncated"):
            break
        key_marker = page.get("NextKeyMarker")
        version_marker = page.get("NextVersionIdMarker")


def cmd_purge(args: argparse.Namespace) -> None:
    config = load_config(Path(args.config))
    check_approval(args.approve, f"PURGE-{config['deployment']}")
    validate_remote_ownership(config)
    validate_bucket_tags(config, config["state_bucket"])
    if bucket_exists(config, config["workspace_bucket"]):
        validate_bucket_tags(config, config["workspace_bucket"])
    for bucket in [config["workspace_bucket"], config["state_bucket"]]:
        if bucket_exists(config, bucket):
            empty_versioned_bucket(config, bucket)
            aws(config, "s3api", "delete-bucket", "--bucket", bucket)
            print(f"Permanently deleted s3://{bucket}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    init = commands.add_parser("init", help="Create setup.toml from read-only AWS discovery")
    init.add_argument("--output", default="setup.toml")
    init.add_argument("--deployment", required=True)
    init.add_argument("--aws-profile", required=True)
    init.add_argument("--region", default="eu-north-1")
    init.add_argument("--method", choices=["terraform", "cli"], default="terraform")
    init.add_argument("--image-tag", default="latest")
    init.add_argument("--app-domain", required=True)
    init.add_argument("--preview-domain", required=True)
    init.add_argument("--dns-provider", choices=["route53", "external"], default="route53")
    init.add_argument("--dns-zone", required=True)
    init.add_argument("--route53-zone-id")
    init.add_argument("--api-cidr", required=True)
    init.add_argument("--force", action="store_true")
    init.set_defaults(func=cmd_init)

    for name, function in [
        ("preflight", cmd_preflight),
        ("render", cmd_render),
        ("plan", cmd_plan),
        ("apply", cmd_apply),
        ("verify", cmd_verify),
        ("secrets-env", cmd_secrets_env),
        ("finalize-dns", cmd_finalize_dns),
        ("destroy-plan", cmd_destroy_plan),
        ("destroy", cmd_destroy),
        ("purge", cmd_purge),
    ]:
        command = commands.add_parser(name)
        command.add_argument("--config", default="setup.toml")
        if name not in {"preflight", "secrets-env", "finalize-dns", "purge"}:
            command.add_argument("--work-dir")
        if name in {"apply", "destroy", "purge", "finalize-dns"}:
            command.add_argument("--approve")
        if name == "verify":
            command.add_argument("--output")
        if name == "secrets-env":
            command.add_argument("--output", default=".env.infrastructure")
            command.add_argument("--force", action="store_true")
            command.add_argument("--work-dir")
        if name in {"finalize-dns", "purge"}:
            command.add_argument("--work-dir")
        command.set_defaults(func=function)

    bootstrap_plan = commands.add_parser("bootstrap-plan")
    bootstrap_plan.add_argument("--config", default="setup.toml")
    bootstrap_plan.add_argument("--output", default="bootstrap-plan.json")
    bootstrap_plan.set_defaults(func=cmd_bootstrap_plan)

    bootstrap = commands.add_parser("bootstrap")
    bootstrap.add_argument("--config", default="setup.toml")
    bootstrap.add_argument("--plan", default="bootstrap-plan.json")
    bootstrap.add_argument("--approve")
    bootstrap.set_defaults(func=cmd_bootstrap)
    return root


def main() -> int:
    try:
        args = parser().parse_args()
        args.func(args)
        return 0
    except ProvisionError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("cancelled", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
