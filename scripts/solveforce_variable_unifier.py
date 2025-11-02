import re
import hashlib
import json
from datetime import datetime

SOLVEFORCE_NAMESPACE_PREFIX = "SFX"

# Accepted modules and scopes
ALLOWED_MODULES = [
    "API",
    "DB",
    "NET",
    "FPGA",
    "AI",
    "MMLU",
    "DNI",
    "IOT",
    "ETHICS",
]
ALLOWED_SCOPES = ["GLOBAL", "LOCAL", "SECURE", "TEMP", "VOLATILE"]

STANDARD_PATTERN = re.compile(
    rf"^{SOLVEFORCE_NAMESPACE_PREFIX}_[A-Z]+_[A-Z]+_[A-Z0-9_]+$"
)


def generate_codex_hash(variable_name: str) -> str:
    """Return the SHA-256 hex digest for the variable."""
    return hashlib.sha256(variable_name.encode("utf-8")).hexdigest()


def validate_variable_name(var_name: str) -> dict:
    """Validate a variable name against the SolveForce pattern."""
    is_match = bool(STANDARD_PATTERN.match(var_name))
    codex_hash = generate_codex_hash(var_name)
    return {
        "original": var_name,
        "valid": is_match,
        "codex_hash": codex_hash[:12],
        "timestamp": datetime.utcnow().isoformat(),
    }


def unify_variable(var_name: str, module: str = "API", scope: str = "GLOBAL") -> dict:
    """Translate arbitrary variable names into SolveForce standard."""
    module = module.upper()
    scope = scope.upper()

    if module not in ALLOWED_MODULES:
        raise ValueError(f"Invalid module '{module}'. Allowed: {ALLOWED_MODULES}")
    if scope not in ALLOWED_SCOPES:
        raise ValueError(f"Invalid scope '{scope}'. Allowed: {ALLOWED_SCOPES}")

    clean = re.sub(r"[^a-zA-Z0-9]", "_", var_name).upper()
    clean = re.sub(r"_+", "_", clean).strip("_")
    unified = f"{SOLVEFORCE_NAMESPACE_PREFIX}_{module}_{scope}_{clean}"
    return validate_variable_name(unified)


def process_variable_map(var_list):
    """Batch process variable names."""
    unified_vars = [unify_variable(v) for v in var_list]
    return json.dumps(unified_vars, indent=4)


def demo():
    demo_vars = [
        "dbTimeout",
        "api_key",
        "mmluWeightVector",
        "delta_phi_std",
        "Omega_new",
        "BioFieldPattern",
        "ethicsConsensusScore",
        "user_name",
    ]

    print("Unifying Variables to SolveForce Standards...\n")
    print(process_variable_map(demo_vars))


if __name__ == "__main__":
    demo()
