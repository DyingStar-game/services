#!/usr/bin/env python3
"""Merge the per-service OpenAPI specs into a single aggregate document.

Reads ``<service>/openapi.yaml`` for every service below and writes the aggregated
``openapi.yaml`` at the repository root, covering **all** endpoints (public, player,
``/api/internal/*`` and ``/api/admin/*``).

Merge rules
-----------
* Every operation is tagged with its service (shared public endpoints use ``public``).
* Component names (schemas, parameters, responses, …) are prefixed with ``<service>_``
  and the matching ``$ref`` values are rewritten, to avoid collisions.
* Each path item carries a ``servers`` entry pointing at its owning service(s).
* Paths served by several services on the same method are merged: their tags are
  unioned, the path lists every involved server and the success response schema becomes
  a ``oneOf`` of the services' schemas.
* The shared ``/api/health`` and ``/openapi.yaml`` endpoints are de-duplicated.

Usage: ``python3 scripts/merge-openapi.py`` (requires PyYAML).
"""
from __future__ import annotations

import copy
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
SERVICES = ["mission", "social", "economie", "inventory", "market"]
METHODS = {"get", "post", "put", "patch", "delete", "head", "options", "trace"}
# Endpoints served identically by every service: kept once under the `public` tag.
PUBLIC_PATHS = {"/api/health", "/openapi.yaml", "/api/openapi.yaml"}
# Component sections whose keys are namespaced per service.
PREFIXED_SECTIONS = {"schemas", "parameters", "responses", "requestBodies", "headers", "examples"}


def load_spec(service: str) -> dict:
    path = ROOT / service / "openapi.yaml"
    if not path.exists():
        sys.exit(f"missing spec: {path}")
    with path.open(encoding="utf-8") as handle:
        return yaml.safe_load(handle)


def prefix_refs(node, service: str) -> None:
    """Rewrite every ``#/components/<section>/<name>`` ref to the namespaced name."""
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str) and value.startswith("#/components/"):
                parts = value.split("/")
                if len(parts) == 4 and parts[2] in PREFIXED_SECTIONS:
                    node[key] = f"#/components/{parts[2]}/{service}_{parts[3]}"
            else:
                prefix_refs(value, service)
    elif isinstance(node, list):
        for item in node:
            prefix_refs(item, service)


def namespaced_components(spec: dict, service: str) -> dict:
    """Return the spec components with per-service prefixes (securitySchemes excluded)."""
    out: dict = {}
    for section, items in (spec.get("components") or {}).items():
        if section == "securitySchemes" or not isinstance(items, dict):
            continue
        out[section] = {f"{service}_{name}": copy.deepcopy(value) for name, value in items.items()}
    return out


def success_response(op: dict):
    """First 2xx response carrying a JSON schema, as ``(code, response, schema)``."""
    for code in ("200", "201", "202", "203", "204"):
        response = (op.get("responses") or {}).get(code)
        if isinstance(response, dict):
            schema = ((response.get("content") or {}).get("application/json") or {}).get("schema")
            if schema is not None:
                return code, response, schema
    return None, None, None


def merge_operations(base: dict, other: dict, other_service: str) -> dict:
    """Merge a second service's operation for the same path+method into ``base``."""
    base["tags"] = sorted(set(base.get("tags", [])) | set(other.get("tags", [])))
    note = f"Also served by the {other_service} service on the same path."
    existing = base.get("description")
    base["description"] = f"{existing.strip()} {note}" if existing else note

    base_code, _, base_schema = success_response(base)
    _, _, other_schema = success_response(other)
    if base_schema is not None and other_schema is not None:
        if isinstance(base_schema.get("oneOf"), list):
            base_schema["oneOf"].append(other_schema)
        else:
            base["responses"][base_code]["content"]["application/json"]["schema"] = {
                "oneOf": [base_schema, other_schema]
            }
    # Keep response codes only present on the other service.
    for code, response in (other.get("responses") or {}).items():
        base.setdefault("responses", {}).setdefault(code, response)
    return base


def param_key(param: dict) -> str:
    """Stable identity for a (path-level) parameter."""
    if "$ref" in param:
        return param["$ref"]
    return f"{param.get('in')}:{param.get('name')}"


def main() -> None:
    specs = {service: load_spec(service) for service in SERVICES}

    merged_paths: dict = {}
    path_services: dict = {}          # path -> ordered owning services
    path_parameters: dict = {}        # path -> ordered path-level parameters
    merged_components: dict = {"securitySchemes": {}}

    for service in SERVICES:
        spec = specs[service]
        components = namespaced_components(spec, service)
        prefix_refs(components, service)
        for section, items in components.items():
            merged_components.setdefault(section, {}).update(items)
        for name, scheme in (spec.get("components", {}).get("securitySchemes") or {}).items():
            merged_components["securitySchemes"].setdefault(name, copy.deepcopy(scheme))

        for path, item in (spec.get("paths") or {}).items():
            is_public = path in PUBLIC_PATHS
            if is_public and path in merged_paths:
                continue

            # Path-level parameters (union across services).
            bucket = path_parameters.setdefault(path, [])
            seen = {param_key(p) for p in bucket}
            for param in item.get("parameters", []) or []:
                param = copy.deepcopy(param)
                prefix_refs(param, service)
                if param_key(param) not in seen:
                    bucket.append(param)
                    seen.add(param_key(param))

            target = merged_paths.setdefault(path, {})
            path_services.setdefault(path, [])
            if service not in path_services[path]:
                path_services[path].append(service)

            for method, op in item.items():
                if method not in METHODS or not isinstance(op, dict):
                    continue
                op = copy.deepcopy(op)
                prefix_refs(op, service)
                op["tags"] = ["public"] if is_public else [service]
                if is_public:
                    op["security"] = []
                if method in target:
                    target[method] = merge_operations(target[method], op, service)
                else:
                    target[method] = op

    # Attach path-level parameters and per-path servers.
    for path, item in merged_paths.items():
        if path_parameters.get(path):
            item["parameters"] = path_parameters[path]
        if path not in PUBLIC_PATHS:
            item["servers"] = [
                {"url": f"http://{service}:3000", "description": service}
                for service in path_services[path]
            ]

    document = {
        "openapi": "3.1.0",
        "info": {
            "title": "DyingStar — API (tous services)",
            "version": "0.1.0",
            "description": (
                "Document agrege (fusion) de **tous** les endpoints des services mission, social, "
                "economie, inventory et market : public, joueur, `/api/internal/*` (serveur de jeu, "
                "tokens de service) et `/api/admin/*`.\n\n"
                "Chaque service expose aussi sa propre spec sur `GET /openapi.yaml`. Chaque operation "
                "porte son service (tag) et l'override `servers` du chemin (les services sont des "
                "processus distincts partageant le prefixe `/api`).\n\n"
                "Authentification : JWT Keycloak joueur (`bearerAuth`) ou token de service "
                "(`serviceAuth`, `/api/internal/*`). En dev uniquement, `AUTH_DEV_BYPASS=true` / "
                "`INTERNAL_DEV_BYPASS=true` acceptent les en-tetes `X-Player-Id` / `X-Internal-Key`.\n\n"
                "Erreurs : { error, message, status }."
            ),
        },
        "tags": [{"name": "public", "description": "Non authentifie (partage)"}]
        + [
            {
                "name": service,
                "description": f"Service {service}",
                "servers": [{"url": f"http://{service}:3000"}],
            }
            for service in SERVICES
        ],
        "security": [{"bearerAuth": []}],
        "paths": {path: merged_paths[path] for path in sorted(merged_paths)},
        "components": merged_components,
    }

    out = ROOT / "openapi.yaml"
    with out.open("w", encoding="utf-8") as handle:
        yaml.dump(
            document,
            handle,
            allow_unicode=True,
            sort_keys=False,
            default_flow_style=False,
            width=120,
        )
    print(f"wrote {out} ({len(merged_paths)} paths, {len(merged_components.get('schemas', {}))} schemas)")


if __name__ == "__main__":
    main()
