from __future__ import annotations

import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = Path(__file__).parents[1] / "scripts" / "provision.py"
SPEC = importlib.util.spec_from_file_location("provision", SCRIPT)
assert SPEC and SPEC.loader
provision = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provision)


def config(method: str = "terraform") -> dict[str, object]:
    return {
        "schema_version": 1,
        "deployment": "moatless-dev",
        "provider": "aws",
        "compute_mode": "fargate",
        "provisioning_method": method,
        "region": "eu-north-1",
        "aws_profile": "test",
        "cluster_version": "1.35",
        "image_tag": "latest",
        "app_namespace": "moatless",
        "sandbox_namespace": "moatless-sandboxes",
        "app_domain": "moatless.example.com",
        "preview_domain": "preview.example.com",
        "dns_provider": "route53",
        "dns_zone": "example.com",
        "route53_zone_id": "ZEXAMPLE",
        "vpc_cidr": "10.42.0.0/16",
        "api_public_access_cidrs": ["203.0.113.10/32"],
        "availability_zones": ["eu-north-1a", "eu-north-1b"],
        "state_bucket": "moatless-dev-state-123456789012",
        "workspace_bucket": "moatless-dev-workspace-123456789012",
        "tags": {"Environment": "dev"},
    }


def toml(config_value: dict[str, object]) -> str:
    lines: list[str] = []
    tags = config_value.get("tags", {})
    for key, value in config_value.items():
        if key == "tags":
            continue
        if isinstance(value, str):
            lines.append(f'{key} = "{value}"')
        elif isinstance(value, list):
            rendered = ", ".join(f'"{item}"' for item in value)
            lines.append(f"{key} = [{rendered}]")
        else:
            lines.append(f"{key} = {value}")
    lines.append("[tags]")
    for key, value in tags.items():
        lines.append(f'{key} = "{value}"')
    return "\n".join(lines) + "\n"


class ProvisionTests(unittest.TestCase):
    def test_accepts_supported_contract(self) -> None:
        provision.validate_config(config())

    def test_accepts_latest_image_tag(self) -> None:
        provision.validate_config(config())

    def test_accepts_external_dns_for_terraform(self) -> None:
        value = config()
        value["dns_provider"] = "external"
        value["route53_zone_id"] = ""
        provision.validate_config(value)

    def test_rejects_external_dns_for_cli_method(self) -> None:
        value = config("cli")
        value["dns_provider"] = "external"
        value["route53_zone_id"] = ""
        with self.assertRaisesRegex(provision.ProvisionError, "terraform"):
            provision.validate_config(value)

    def test_rejects_domain_outside_dns_zone(self) -> None:
        value = config()
        value["app_domain"] = "moatless.example.net"
        with self.assertRaisesRegex(provision.ProvisionError, "outside DNS zone"):
            provision.validate_config(value)

    def test_rejects_public_api_open_to_world(self) -> None:
        value = config()
        value["api_public_access_cidrs"] = ["0.0.0.0/0"]
        with self.assertRaisesRegex(provision.ProvisionError, "cannot be open"):
            provision.validate_config(value)

    def test_rejects_invalid_networks_with_actionable_errors(self) -> None:
        value = config()
        value["vpc_cidr"] = "not-a-network"
        with self.assertRaisesRegex(provision.ProvisionError, "vpc_cidr is invalid"):
            provision.validate_config(value)

        value = config()
        value["api_public_access_cidrs"] = ["not-a-cidr"]
        with self.assertRaisesRegex(provision.ProvisionError, "invalid API access CIDR"):
            provision.validate_config(value)

    def test_parses_helm_three_and_four_versions(self) -> None:
        self.assertEqual(
            provision.parse_version("helm", 'version.BuildInfo{Version:"v4.1.1"}'),
            (4, 1, 1),
        )
        self.assertEqual(
            provision.parse_version("helm", 'version.BuildInfo{Version:"v3.20.4"}'),
            (3, 20, 4),
        )

    def test_subnet_layout_has_no_overlap(self) -> None:
        result = provision.subnet_cidrs("10.42.0.0/16")
        subnets = [item for group in result.values() for item in group]
        self.assertEqual(len(subnets), len(set(subnets)))
        self.assertEqual(result["private"], ["10.42.0.0/20", "10.42.16.0/20"])

    def test_render_keeps_methods_separate(self) -> None:
        for method in ["terraform", "cli"]:
            with self.subTest(method=method), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                config_path = root / "setup.toml"
                config_path.write_text(toml(config(method)))
                output = root / "work"
                args = SimpleNamespace(config=str(config_path), work_dir=str(output))

                provision.cmd_render(args)

                self.assertEqual((output / provision.METHOD_MARKER).read_text().strip(), method)
                self.assertTrue((output / "render.json").exists())
                before = provision.deployment_artifact_hash(config(method), output)
                if method == "terraform":
                    artifact = output / "terraform" / "backend.hcl"
                else:
                    artifact = output / "cli" / "foundation-parameters.json"
                self.assertTrue(artifact.exists())
                artifact.write_text(artifact.read_text() + "\n")
                self.assertNotEqual(
                    before,
                    provision.deployment_artifact_hash(config(method), output),
                )

    def test_render_removes_stale_generated_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config_path = root / "setup.toml"
            config_path.write_text(toml(config()))
            output = root / "work"
            args = SimpleNamespace(config=str(config_path), work_dir=str(output))

            provision.cmd_render(args)
            stale_terraform = output / "terraform" / "removed-resource.tf"
            stale_kubernetes = output / "kubernetes" / "removed-resource.yaml"
            stale_terraform.write_text("stale")
            stale_kubernetes.write_text("stale")

            provision.cmd_render(args)

            self.assertFalse(stale_terraform.exists())
            self.assertFalse(stale_kubernetes.exists())
            render = json.loads((output / "render.json").read_text())
            self.assertEqual(render["assetSha256"], provision.source_asset_hash("terraform"))

    def test_render_refuses_unowned_generated_directories(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config_path = root / "setup.toml"
            config_path.write_text(toml(config()))
            output = root / "work"
            (output / "terraform").mkdir(parents=True)
            sentinel = output / "terraform" / "do-not-delete"
            sentinel.write_text("user data")
            args = SimpleNamespace(config=str(config_path), work_dir=str(output))

            with self.assertRaisesRegex(provision.ProvisionError, "unowned"):
                provision.cmd_render(args)

            self.assertEqual(sentinel.read_text(), "user data")

    def test_work_dir_refuses_the_current_directory(self) -> None:
        args = SimpleNamespace(work_dir=str(Path.cwd()))
        with self.assertRaisesRegex(provision.ProvisionError, "dedicated child"):
            provision.work_dir(args, config())

    def test_private_output_is_mode_0600_and_rejects_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "secret.env"

            provision.write_private_text(output, "SECRET=value\n")

            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            target = root / "target"
            target.write_text("preserve")
            link = root / "link"
            os.symlink(target, link)
            with self.assertRaisesRegex(provision.ProvisionError, "symlink"):
                provision.write_private_text(link, "overwrite")
            self.assertEqual(target.read_text(), "preserve")

    def test_update_kubeconfig_restores_the_previous_context(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], **_kwargs: object) -> str:
            commands.append(command)
            if command == ["kubectl", "config", "current-context"]:
                return "existing-context"
            if command == ["kubectl", "config", "get-contexts", "-o", "name"]:
                return "existing-context\nmoatless-dev-eu-north-1"
            return ""

        with (
            mock.patch.object(provision, "run", side_effect=fake_run),
            mock.patch.object(provision, "aws"),
        ):
            context = provision.update_kubeconfig(config())

        self.assertEqual(context, "moatless-dev-eu-north-1")
        self.assertIn(
            ["kubectl", "config", "use-context", "existing-context"],
            commands,
        )

    def test_bootstrap_records_state_ownership_before_workspace_setup(self) -> None:
        value = config()
        args = SimpleNamespace(
            approve="BOOTSTRAP-moatless-dev",
            config="setup.toml",
            plan="bootstrap-plan.json",
        )
        events: list[tuple[str, str]] = []

        def configure(_config: dict[str, object], bucket: str, _purpose: str) -> None:
            events.append(("configure", bucket))

        def marker(_config: dict[str, object]) -> None:
            events.append(("marker", value["state_bucket"]))

        with (
            mock.patch.object(provision, "load_config", return_value=value),
            mock.patch.object(provision, "validate_plan"),
            mock.patch.object(provision, "bucket_exists", side_effect=[False, False]),
            mock.patch.object(provision, "configure_bucket", side_effect=configure),
            mock.patch.object(provision, "put_method_marker", side_effect=marker),
        ):
            provision.cmd_bootstrap(args)

        self.assertEqual(
            events,
            [
                ("configure", value["state_bucket"]),
                ("marker", value["state_bucket"]),
                ("configure", value["workspace_bucket"]),
            ],
        )

    def test_bootstrap_repairs_a_tagged_state_bucket_missing_its_marker(self) -> None:
        value = config()
        args = SimpleNamespace(
            approve="BOOTSTRAP-moatless-dev",
            config="setup.toml",
            plan="bootstrap-plan.json",
        )

        with (
            mock.patch.object(provision, "load_config", return_value=value),
            mock.patch.object(provision, "validate_plan"),
            mock.patch.object(provision, "bucket_exists", side_effect=[True, False]),
            mock.patch.object(provision, "validate_bucket_tags") as validate_tags,
            mock.patch.object(provision, "read_method_marker", return_value=None),
            mock.patch.object(provision, "validate_remote_ownership") as validate_ownership,
            mock.patch.object(provision, "configure_bucket"),
            mock.patch.object(provision, "put_method_marker") as put_marker,
        ):
            provision.cmd_bootstrap(args)

        validate_tags.assert_called_once_with(value, value["state_bucket"])
        validate_ownership.assert_not_called()
        put_marker.assert_called_once_with(value)


if __name__ == "__main__":
    unittest.main()
