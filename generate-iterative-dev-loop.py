#!/usr/bin/env python3
"""Generate Mozart score YAML for an iterative development loop.

Produces a score with:
- Pre-loop: Intent alignment + preprocessing
- Loop (N cycles): Executive → Manager → Investigation → Test Design (TDD) →
  Implementation → Documentation → TDF Review → Synthesizer →
  Adversarial Validation → Memory Consolidation (dreamers)
- Post-loop: 5-stage documentation pipeline

Usage:
    python scripts/generate-iterative-dev-loop.py config.yaml -o scores/my-feature.yaml
    python scripts/generate-iterative-dev-loop.py config.yaml --dry-run
    python scripts/generate-iterative-dev-loop.py config.yaml -o scores/quick.yaml --cycles 5
"""

from __future__ import annotations

import argparse
import copy
import sys
from pathlib import Path
from typing import Any

import yaml


# ═══════════════════════════════════════════════════════════════════════════
# DEFAULT CONFIGURATION
# ═══════════════════════════════════════════════════════════════════════════

STAGES_PER_CYCLE = 10
PRE_LOOP_STAGES = 2
POST_LOOP_STAGES = 5

DEFAULT_AGENTS = {
    "workers": 3,
    "qa": 3,
    "docs": 2,
    "executives": 1,
    "managers": 1,
    "reviewers": 5,
    "antagonists": 2,
    "dreamers": 5,
}

DEFAULT_BACKEND = {
    "type": "claude_cli",
    "skip_permissions": True,
    "disable_mcp": True,
    "timeout_seconds": 3600,
    "output_format": "text",
}

DEFAULT_WORKER_PERSONAS = {
    1: {
        "name": "COMP",
        "voice": (
            "You are analytical. You focus on logic, patterns, and systematic "
            "correctness. You think in structures, algorithms, and edge cases."
        ),
        "focus": "Architecture, algorithms, data structures, edge cases",
    },
    2: {
        "name": "SCI",
        "voice": (
            "You are empirical. You demand evidence, tests, and measurable "
            "outcomes. Nothing is true until it's tested."
        ),
        "focus": "Testing, benchmarks, validation, reproducibility",
    },
    3: {
        "name": "CULT",
        "voice": (
            "You are contextual. You care about why things exist and who they "
            "serve. Code is for humans first, machines second."
        ),
        "focus": "API design, documentation, conventions, user experience",
    },
}

DEFAULT_QA_PERSONAS = {
    1: {
        "name": "QA-Logic",
        "voice": (
            "You are a logic-focused test engineer. You find the edge cases "
            "everyone else misses. Off-by-one, boundary conditions, type "
            "coercion, null handling — these are your territory."
        ),
        "focus": "Edge cases, boundary conditions, type safety, error paths",
    },
    2: {
        "name": "QA-Evidence",
        "voice": (
            "You are a coverage-focused test engineer. You write property-based "
            "tests, measure coverage, and build regression suites. You prove "
            "things work through statistical evidence, not spot checks."
        ),
        "focus": "Coverage metrics, property-based tests, regression suites",
    },
    3: {
        "name": "QA-Integration",
        "voice": (
            "You are an integration-focused test engineer. You test how "
            "components work together, verify user flows end-to-end, and "
            "check that error messages are actually helpful."
        ),
        "focus": "Integration tests, user flow tests, API contract tests, error UX",
    },
}

DEFAULT_DOC_PERSONAS = {
    1: {
        "name": "Doc-Technical",
        "voice": (
            "You are a technical writer who ensures API documentation is "
            "accurate, complete, and matches the actual code behavior."
        ),
        "focus": "API docs, code examples, configuration reference, architecture",
    },
    2: {
        "name": "Doc-User",
        "voice": (
            "You are a user-facing writer who ensures guides, tutorials, "
            "and README content help people actually use the software."
        ),
        "focus": "Guides, tutorials, quickstart, error troubleshooting, examples",
    },
}

DEFAULT_MANAGER_PERSONAS = {
    1: {
        "name": "META",
        "voice": (
            "You are the team lead. You see the whole picture and direct resources "
            "where they matter most. You track what each team member is good at, "
            "what they struggle with, and assign work accordingly. You translate "
            "intent goals into per-agent directives."
        ),
        "focus": "Coordination, prioritization, intent alignment, team performance",
    },
}

DEFAULT_EXECUTIVE_PERSONAS = {
    1: {
        "name": "VISION",
        "voice": (
            "You are the executive. You hold the long-term vision for the entire "
            "project. You read the full specification and track the total company "
            "trajectory. You give clear, short directives. You will not say DONE "
            "until the full spec vision is built — even if everyone else says done."
        ),
        "focus": "Long-term vision, spec completeness, strategic direction, total trajectory",
    },
}

DEFAULT_REVIEWER_PERSONAS = {
    1: {
        "name": "COMP-Review",
        "voice": "You are the analytical reviewer. Challenge the logic.",
        "focus": "Correctness, edge cases, algorithmic soundness",
    },
    2: {
        "name": "SCI-Review",
        "voice": "You are the empirical reviewer. Demand evidence.",
        "focus": "Test coverage, benchmarks, performance evidence",
    },
    3: {
        "name": "CULT-Review",
        "voice": "You are the contextual reviewer. Check cultural fit.",
        "focus": "Conventions, readability, documentation, API ergonomics",
    },
    4: {
        "name": "EXP-Review",
        "voice": "You are the experiential reviewer. Trust your instincts.",
        "focus": "Developer experience, friction points, 'this feels wrong' moments",
    },
    5: {
        "name": "META-Review",
        "voice": "You are the process reviewer. Evaluate the team's approach.",
        "focus": "Architecture coherence, technical debt, future maintainability",
    },
}

DEFAULT_ANTAGONIST_PERSONAS = {
    1: {
        "name": "Naive User",
        "voice": (
            "You have never seen this project before. You will try to use it "
            "with zero context, following only whatever documentation exists."
        ),
        "focus": "Onboarding, error messages, documentation gaps, obvious failures",
    },
    2: {
        "name": "Power User",
        "voice": (
            "You are an expert who will push this to its limits. You know "
            "every trick and you will find every weakness."
        ),
        "focus": "Edge cases, performance under load, API abuse, security",
    },
}

DEFAULT_DREAMER_PERSONAS = {
    1: {
        "name": "Dreamer-Workers",
        "voice": "You consolidate memories for the worker agents.",
        "focus": "Worker memory tiering, growth trajectory preservation",
    },
    2: {
        "name": "Dreamer-QA",
        "voice": "You consolidate memories for the QA agents.",
        "focus": "QA memory tiering, test strategy evolution tracking",
    },
    3: {
        "name": "Dreamer-Lead",
        "voice": "You consolidate memories for the lead and documentation agents.",
        "focus": "Management decisions, documentation progress tracking",
    },
    4: {
        "name": "Dreamer-Reviewers",
        "voice": "You consolidate memories for the reviewer agents.",
        "focus": "Review pattern evolution, cross-domain consistency",
    },
    5: {
        "name": "Dreamer-Collective",
        "voice": "You consolidate the collective shared memory.",
        "focus": "Status pruning, decision archiving, coordination freshness",
    },
}


# ═══════════════════════════════════════════════════════════════════════════
# CONFIG LOADING
# ═══════════════════════════════════════════════════════════════════════════


def load_config(config_path: str, cycle_override: int | None = None) -> dict[str, Any]:
    """Load config YAML and merge with defaults."""
    with open(config_path) as f:
        raw = yaml.safe_load(f) or {}

    config: dict[str, Any] = {}

    # Required fields
    config["name"] = raw.get("name", "iterative-dev-loop")
    config["workspace"] = raw.get("workspace", "./workspaces/dev-loop")
    config["spec_dir"] = raw.get("spec_dir", "./specs/")
    config["cycles"] = cycle_override or raw.get("cycles", 20)

    # Agent counts
    agents = raw.get("agents", {})
    config["agents"] = {
        k: agents.get(k, v) for k, v in DEFAULT_AGENTS.items()
    }

    # Personas — deep merge with defaults
    personas = raw.get("personas", {})
    config["worker_personas"] = _merge_personas(
        DEFAULT_WORKER_PERSONAS, personas.get("worker_personas"), config["agents"]["workers"]
    )
    config["qa_personas"] = _merge_personas(
        DEFAULT_QA_PERSONAS, personas.get("qa_personas"), config["agents"]["qa"]
    )
    config["doc_personas"] = _merge_personas(
        DEFAULT_DOC_PERSONAS, personas.get("doc_personas"), config["agents"]["docs"]
    )
    config["manager_personas"] = _merge_personas(
        DEFAULT_MANAGER_PERSONAS, personas.get("manager_personas"), config["agents"]["managers"]
    )
    # Backward compat: if lead_persona is set and manager_personas isn't, use it for manager 1
    if personas.get("lead_persona") and not personas.get("manager_personas"):
        config["manager_personas"][1] = copy.deepcopy(personas["lead_persona"])
    config["executive_personas"] = _merge_personas(
        DEFAULT_EXECUTIVE_PERSONAS, personas.get("executive_personas"), config["agents"]["executives"]
    )
    config["reviewer_personas"] = _merge_personas(
        DEFAULT_REVIEWER_PERSONAS, personas.get("reviewer_personas"), config["agents"]["reviewers"]
    )
    config["antagonist_personas"] = _merge_personas(
        DEFAULT_ANTAGONIST_PERSONAS, personas.get("antagonist_personas"), config["agents"]["antagonists"]
    )
    config["dreamer_personas"] = _merge_personas(
        DEFAULT_DREAMER_PERSONAS, personas.get("dreamer_personas"), config["agents"]["dreamers"]
    )

    # Spec quality level
    config["spec_quality"] = raw.get("spec_quality", "detailed")

    # Hierarchy — explicit pairings and reporting lines
    hierarchy = raw.get("hierarchy", {})
    config["hierarchy"] = _build_hierarchy(hierarchy, config["agents"])

    # Backend
    backend = raw.get("backend", {})
    config["backend"] = {k: backend.get(k, v) for k, v in DEFAULT_BACKEND.items()}

    # Prelude
    config["prelude"] = raw.get("prelude", [])

    # Custom validations
    config["custom_validations"] = raw.get("custom_validations", [])

    # Parallel config
    config["parallel"] = raw.get("parallel", {"enabled": True, "max_concurrent": 5})

    # Retry config
    config["retry"] = raw.get("retry", {
        "max_retries": 2,
        "base_delay_seconds": 15,
        "max_completion_attempts": 3,
        "completion_threshold_percent": 100,
    })

    # Rate limit
    config["rate_limit"] = raw.get("rate_limit", {
        "wait_minutes": 60,
        "max_waits": 12,
    })

    # Stale detection
    config["stale_detection"] = raw.get("stale_detection", {
        "enabled": True,
        "idle_timeout_seconds": 1800,
    })

    return config


def _merge_personas(
    defaults: dict[int, dict[str, str]],
    overrides: dict[int, dict[str, str]] | None,
    count: int,
) -> dict[int, dict[str, str]]:
    """Merge persona overrides with defaults, respecting agent count."""
    result = {}
    for i in range(1, count + 1):
        base = copy.deepcopy(defaults.get(i, defaults.get(1, {})))
        if overrides and i in overrides:
            base.update(overrides[i])
        result[i] = base
    return result


def _build_hierarchy(
    raw_hierarchy: dict[str, Any],
    agents: dict[str, int],
) -> dict[str, Any]:
    """Build hierarchy config from explicit config or auto-compute defaults.

    Returns a dict with:
        reports: dict[int, dict]  — manager_id → {workers: [...], qa: [...], docs: [...]}
        pairings: dict with qa_to_workers, doc_to_workers mappings
        dreamer_groups: dict[int, list[str]]
    """
    managers = agents["managers"]
    workers = agents["workers"]
    qa = agents["qa"]
    docs = agents["docs"]
    reviewers = agents["reviewers"]

    # Reports: which agents each manager is responsible for
    if "reports" in raw_hierarchy:
        reports = {int(k): v for k, v in raw_hierarchy["reports"].items()}
    else:
        # Auto-compute: round-robin workers/qa/docs across managers
        reports: dict[int, dict[str, list[int]]] = {
            m: {"workers": [], "qa": [], "docs": []} for m in range(1, managers + 1)
        }
        for i in range(1, workers + 1):
            mgr = ((i - 1) % managers) + 1
            reports[mgr]["workers"].append(i)
        for i in range(1, qa + 1):
            mgr = ((i - 1) % managers) + 1
            reports[mgr]["qa"].append(i)
        for i in range(1, docs + 1):
            mgr = ((i - 1) % managers) + 1
            reports[mgr]["docs"].append(i)

    # Pairings: QA-to-workers and doc-to-workers
    raw_pairings = raw_hierarchy.get("pairings", {})
    if "qa_to_workers" in raw_pairings:
        qa_to_workers = {int(k): v for k, v in raw_pairings["qa_to_workers"].items()}
    else:
        # Auto-compute: round-robin workers across QA agents
        qa_to_workers: dict[int, list[int]] = {i: [] for i in range(1, qa + 1)}
        for i in range(1, workers + 1):
            qa_idx = ((i - 1) % qa) + 1
            qa_to_workers[qa_idx].append(i)

    if "doc_to_workers" in raw_pairings:
        doc_to_workers = {int(k): v for k, v in raw_pairings["doc_to_workers"].items()}
    else:
        # Auto-compute: round-robin workers across doc agents
        doc_to_workers: dict[int, list[int]] = {i: [] for i in range(1, docs + 1)}
        for i in range(1, workers + 1):
            doc_idx = ((i - 1) % docs) + 1
            doc_to_workers[doc_idx].append(i)

    pairings: dict[str, Any] = {
        "qa_to_workers": qa_to_workers,
        "doc_to_workers": doc_to_workers,
    }

    # Reverse lookups: worker → QA, worker → doc
    worker_to_qa: dict[int, list[int]] = {i: [] for i in range(1, workers + 1)}
    for qa_id, worker_ids in qa_to_workers.items():
        for w in worker_ids:
            worker_to_qa[w].append(qa_id)
    pairings["worker_to_qa"] = worker_to_qa

    worker_to_doc: dict[int, list[int]] = {i: [] for i in range(1, workers + 1)}
    for doc_id, worker_ids in doc_to_workers.items():
        for w in worker_ids:
            worker_to_doc[w].append(doc_id)
    pairings["worker_to_doc"] = worker_to_doc

    # Worker/QA/doc → manager reverse lookup
    worker_to_manager: dict[int, int] = {}
    qa_to_manager: dict[int, int] = {}
    doc_to_manager: dict[int, int] = {}
    for mgr_id, mgr_reports in reports.items():
        for w in mgr_reports.get("workers", []):
            worker_to_manager[w] = mgr_id
        for q in mgr_reports.get("qa", []):
            qa_to_manager[q] = mgr_id
        for d in mgr_reports.get("docs", []):
            doc_to_manager[d] = mgr_id
    pairings["worker_to_manager"] = worker_to_manager
    pairings["qa_to_manager"] = qa_to_manager
    pairings["doc_to_manager"] = doc_to_manager

    # Dreamer groups
    if "dreamer_groups" in raw_hierarchy:
        dreamer_groups = {int(k): v for k, v in raw_hierarchy["dreamer_groups"].items()}
    else:
        # Default: 1=workers, 2=QA, 3=executives+managers+docs, 4=reviewers, 5=collective
        dreamer_groups: dict[int, Any] = {}
        group_idx = 1
        dreamer_count = agents["dreamers"]
        executives = agents.get("executives", 1)
        # Workers
        if group_idx <= dreamer_count:
            dreamer_groups[group_idx] = [f"worker-{i}" for i in range(1, workers + 1)]
            group_idx += 1
        # QA
        if group_idx <= dreamer_count:
            dreamer_groups[group_idx] = [f"qa-{i}" for i in range(1, qa + 1)]
            group_idx += 1
        # Executives + Managers + docs
        if group_idx <= dreamer_count:
            dreamer_groups[group_idx] = (
                [f"executive-{i}" for i in range(1, executives + 1)]
                + [f"manager-{i}" for i in range(1, managers + 1)]
                + [f"doc-{i}" for i in range(1, docs + 1)]
            )
            group_idx += 1
        # Reviewers
        if group_idx <= dreamer_count:
            dreamer_groups[group_idx] = [f"reviewer-{i}" for i in range(1, reviewers + 1)]
            group_idx += 1
        # Collective
        if group_idx <= dreamer_count:
            dreamer_groups[group_idx] = "collective"
            group_idx += 1

    return {
        "reports": reports,
        "pairings": pairings,
        "dreamer_groups": dreamer_groups,
    }


def validate_hierarchy(config: dict[str, Any]) -> list[str]:
    """Validate hierarchy config. Returns list of error messages (empty = valid)."""
    errors: list[str] = []
    agents = config["agents"]
    hierarchy = config["hierarchy"]
    reports = hierarchy["reports"]
    pairings = hierarchy["pairings"]

    # Check all workers are assigned to a manager
    assigned_workers: set[int] = set()
    assigned_qa: set[int] = set()
    assigned_docs: set[int] = set()
    for mgr_id, mgr_reports in reports.items():
        for w in mgr_reports.get("workers", []):
            if w in assigned_workers:
                errors.append(f"Worker {w} assigned to multiple managers (including manager {mgr_id})")
            assigned_workers.add(w)
        for q in mgr_reports.get("qa", []):
            if q in assigned_qa:
                errors.append(f"QA {q} assigned to multiple managers (including manager {mgr_id})")
            assigned_qa.add(q)
        for d in mgr_reports.get("docs", []):
            if d in assigned_docs:
                errors.append(f"Doc {d} assigned to multiple managers (including manager {mgr_id})")
            assigned_docs.add(d)

    for i in range(1, agents["workers"] + 1):
        if i not in assigned_workers:
            errors.append(f"Worker {i} has no manager")

    for i in range(1, agents["qa"] + 1):
        if i not in assigned_qa:
            errors.append(f"QA {i} has no manager")

    for i in range(1, agents["docs"] + 1):
        if i not in assigned_docs:
            errors.append(f"Doc {i} has no manager")

    # Check all workers are covered by at least one QA
    qa_covered_workers: set[int] = set()
    for _qa_id, worker_ids in pairings["qa_to_workers"].items():
        for w in worker_ids:
            qa_covered_workers.add(w)

    for i in range(1, agents["workers"] + 1):
        if i not in qa_covered_workers:
            errors.append(f"Worker {i} has no paired QA agent")

    return errors


# ═══════════════════════════════════════════════════════════════════════════
# STAGE COMPUTATION
# ═══════════════════════════════════════════════════════════════════════════


def compute_stages(config: dict[str, Any]) -> dict[str, Any]:
    """Compute total stages, fan_out, dependencies, and skip_when_command.

    Returns a dict with:
        total_stages: int
        fan_out: dict[int, int]
        dependencies: dict[int, list[int]]
        skip_when_command: dict[int, dict]
        stage_map: list of (stage_num, role, cycle) tuples
    """
    cycles = config["cycles"]
    agents = config["agents"]

    total_stages = PRE_LOOP_STAGES + (STAGES_PER_CYCLE * cycles) + POST_LOOP_STAGES

    # Fan-out: keyed by stage number
    fan_out: dict[int, int] = {}
    # Stage 2: preprocessing (workers)
    fan_out[2] = agents["workers"]

    for c in range(1, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)
        # Executive: stage base+1 (fan-out = number of executives)
        if agents["executives"] > 1:
            fan_out[base + 1] = agents["executives"]
        # Manager: stage base+2 (fan-out = number of managers)
        if agents["managers"] > 1:
            fan_out[base + 2] = agents["managers"]
        # Investigation: stage base+3
        fan_out[base + 3] = agents["workers"]
        # Test Design: stage base+4
        fan_out[base + 4] = agents["qa"]
        # Implementation: stage base+5
        fan_out[base + 5] = agents["workers"]
        # Documentation: stage base+6
        fan_out[base + 6] = agents["docs"]
        # TDF Review: stage base+7
        fan_out[base + 7] = agents["reviewers"]
        # Synthesizer: stage base+8 (fan-out 1, no entry needed)
        # Antagonist: stage base+9
        fan_out[base + 9] = agents["antagonists"]
        # Dreamers: stage base+10
        fan_out[base + 10] = agents["dreamers"]

    # Post-loop stages
    post_base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * cycles
    # Doc Manager: post_base+1 (fan-out 1)
    # Doc Analysis: post_base+2
    fan_out[post_base + 2] = agents["workers"] + agents["qa"] + agents["docs"]
    # Doc Plan: post_base+3 (fan-out 1)
    # Doc Execution: post_base+4 (fan-out 1)
    # Doc Review: post_base+5 (fan-out 1)

    # Dependencies: strictly sequential within cycles + cross-cycle links
    dependencies: dict[int, list[int]] = {}
    # Stage 2 depends on stage 1
    dependencies[2] = [1]

    for c in range(1, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)
        # Executive depends on: preprocessing (cycle 1) or previous dreamers
        if c == 1:
            dependencies[base + 1] = [2]  # Executive depends on preprocessing
        else:
            prev_base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 2)
            dependencies[base + 1] = [prev_base + 10]  # Executive depends on prev dreamers

        # Sequential within cycle
        for phase in range(2, 11):  # stages 2-10 within cycle
            dependencies[base + phase] = [base + phase - 1]

    # Post-loop depends on last dreamer stage
    last_dreamer = PRE_LOOP_STAGES + STAGES_PER_CYCLE * cycles  # = base+10 of last cycle
    dependencies[post_base + 1] = [last_dreamer]
    for i in range(2, POST_LOOP_STAGES + 1):
        dependencies[post_base + i] = [post_base + i - 1]

    # Skip_when_command: on stages 2+ within cycles 2+ (executive excluded).
    # Executive (phase 1) always runs — it's the vision gatekeeper.
    # SKIPPED sheets satisfy dependencies in Mozart, so we must skip
    # every stage (except executive) in a completed cycle.
    #
    # STRICT CHECKS: Verdict files must exist AND contain DONE. Missing
    # files mean the previous cycle didn't fully complete (e.g., partial
    # re-run), so we must NOT skip — the cycle needs to run again.
    # The executive verdict is always required (exec always runs).
    skip_when_command: dict[int, dict[str, Any]] = {}
    for c in range(2, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)
        prev_cycle = c - 1
        # Build strict checks for prev cycle verdicts:
        # file must exist AND contain DONE. Missing files mean the cycle
        # didn't complete, so we must NOT skip — require re-run.
        synth_check = (
            f'test -f "{{workspace}}/cycle-{prev_cycle}/verdict-synthesizer.md" && '
            f'grep -q "VERDICT: DONE" "{{workspace}}/cycle-{prev_cycle}/verdict-synthesizer.md"'
        )
        antag_checks = " && ".join(
            f'test -f "{{workspace}}/cycle-{prev_cycle}/verdict-antagonist-{a}.md" && '
            f'grep -q "VERDICT: DONE" "{{workspace}}/cycle-{prev_cycle}/verdict-antagonist-{a}.md"'
            for a in range(1, agents["antagonists"] + 1)
        )
        # Previous cycle's executive verdict must also be DONE
        prev_exec_check = (
            f'test -f "{{workspace}}/cycle-{prev_cycle}/verdict-executive-1.md" && '
            f'grep -q "VERDICT: DONE" "{{workspace}}/cycle-{prev_cycle}/verdict-executive-1.md"'
        )
        # Current cycle's executive verdict is always required (exec always runs)
        exec_check = (
            f'grep -q "VERDICT: DONE" "{{workspace}}/cycle-{c}/verdict-executive-1.md"'
        )
        cmd = f'{exec_check} && {prev_exec_check} && {synth_check} && {antag_checks}'
        # Executive (phase 1) is excluded — always runs
        for phase in range(2, STAGES_PER_CYCLE + 1):
            skip_when_command[base + phase] = {
                "command": cmd,
                "description": f"Skip cycle {c} stage {phase} if all verdicts DONE",
                "timeout_seconds": 10,
            }

    # Expand skip_when_command from stage-keyed to sheet-keyed.
    # Mozart expects skip_when_command keys to be post-expansion sheet
    # numbers, not pre-expansion stage numbers. Build the same
    # stage_to_first_sheet mapping used by compute_cadenzas().
    stage_to_first_sheet: dict[int, int] = {}
    current_sheet = 1
    for stage_num in range(1, total_stages + 1):
        stage_to_first_sheet[stage_num] = current_sheet
        current_sheet += fan_out.get(stage_num, 1)

    expanded_skip: dict[int, dict[str, Any]] = {}
    for stage_num, condition in skip_when_command.items():
        first_sheet = stage_to_first_sheet[stage_num]
        count = fan_out.get(stage_num, 1)
        for i in range(count):
            expanded_skip[first_sheet + i] = condition
    skip_when_command = expanded_skip

    # Build stage map for reference
    stage_map = []
    stage_map.append((1, "intent_alignment", 0))
    stage_map.append((2, "preprocessor", 0))
    for c in range(1, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)
        roles = [
            "executive", "manager", "investigator", "test_designer", "implementer",
            "documenter", "reviewer", "synthesizer", "antagonist", "dreamer",
        ]
        for phase_idx, role in enumerate(roles):
            stage_map.append((base + phase_idx + 1, role, c))
    post_roles = ["doc_manager", "doc_analyst", "doc_planner", "doc_executor", "doc_reviewer"]
    for i, role in enumerate(post_roles):
        stage_map.append((post_base + i + 1, role, 0))

    return {
        "total_stages": total_stages,
        "fan_out": fan_out,
        "dependencies": dependencies,
        "skip_when_command": skip_when_command,
        "stage_map": stage_map,
    }


# ═══════════════════════════════════════════════════════════════════════════
# CADENZA GENERATION
# ═══════════════════════════════════════════════════════════════════════════


def compute_cadenzas(config: dict[str, Any], stage_info: dict[str, Any]) -> dict[int, list[dict[str, str]]]:
    """Compute cadenzas keyed by concrete sheet number (post fan-out expansion).

    Must replicate Mozart's fan-out expansion to map stages → concrete sheets.
    """
    fan_out = stage_info["fan_out"]
    total_stages = stage_info["total_stages"]
    cycles = config["cycles"]
    agents = config["agents"]
    ws = "{{ workspace }}"

    # Build stage-to-sheet mapping: for each stage, compute the first concrete
    # sheet number and the fan-out count
    stage_to_first_sheet: dict[int, int] = {}
    current_sheet = 1
    for stage_num in range(1, total_stages + 1):
        stage_to_first_sheet[stage_num] = current_sheet
        count = fan_out.get(stage_num, 1)
        current_sheet += count

    cadenzas: dict[int, list[dict[str, str]]] = {}

    def _add_cadenza(stage: int, instance: int, file_path: str, as_type: str = "context") -> None:
        """Add a cadenza for a specific stage+instance → concrete sheet."""
        sheet = stage_to_first_sheet[stage] + (instance - 1)
        cadenzas.setdefault(sheet, []).append({"file": file_path, "as": as_type})

    hierarchy = config["hierarchy"]
    pairings = hierarchy["pairings"]
    num_managers = agents["managers"]

    num_executives = agents.get("executives", 1)

    for c in range(1, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)

        # Executive (base+1): own memory + intent brief + board notes + manager-level reports
        # Executive delegates to managers and gets reports from managers — does NOT
        # read individual worker/QA/doc outputs directly.
        exec_stage = base + 1
        for e in range(1, num_executives + 1):
            if c > 1:
                _add_cadenza(exec_stage, e, f"{ws}/memory/executive-{e}.md")
            _add_cadenza(exec_stage, e, f"{ws}/intent-brief.md")
            _add_cadenza(exec_stage, e, f"{ws}/memory/notes-from-the-board.md")
            if c > 1:
                # Cycle 2+: roadmap built in cycle 1, plus manager-level reports
                _add_cadenza(exec_stage, e, f"{ws}/executive-roadmap-{e}.md")
                _add_cadenza(exec_stage, e, f"{ws}/cycle-{c - 1}/synthesis.md")
                for a in range(1, agents["antagonists"] + 1):
                    _add_cadenza(exec_stage, e, f"{ws}/cycle-{c - 1}/antagonist-{a}.md")
                    _add_cadenza(exec_stage, e, f"{ws}/cycle-{c - 1}/verdict-antagonist-{a}.md")
                for m in range(1, num_managers + 1):
                    _add_cadenza(exec_stage, e, f"{ws}/cycle-{c - 1}/assignments-{m}.md")

        # Manager (base+2): own memory + intent brief + executive directives + previous synthesis
        manager_stage = base + 2
        for m in range(1, num_managers + 1):
            if c > 1:
                _add_cadenza(manager_stage, m, f"{ws}/memory/manager-{m}.md")
            _add_cadenza(manager_stage, m, f"{ws}/intent-brief.md")
            for e in range(1, num_executives + 1):
                _add_cadenza(manager_stage, m, f"{ws}/cycle-{c}/executive-directives-{e}.md")
            if c > 1:
                _add_cadenza(manager_stage, m, f"{ws}/cycle-{c - 1}/synthesis.md")

        # Investigation (base+3): each worker gets their manager's assignments
        inv_stage = base + 3
        for i in range(1, agents["workers"] + 1):
            mgr = pairings["worker_to_manager"].get(i, 1)
            _add_cadenza(inv_stage, i, f"{ws}/cycle-{c}/assignments-{mgr}.md")

        # Test Design (base+4): QA gets their manager's assignments + paired workers' investigations
        test_stage = base + 4
        for i in range(1, agents["qa"] + 1):
            mgr = pairings["qa_to_manager"].get(i, 1)
            _add_cadenza(test_stage, i, f"{ws}/cycle-{c}/assignments-{mgr}.md")
            # Get investigation briefs from ALL paired workers
            paired_workers = pairings["qa_to_workers"].get(i, [i])
            for w in paired_workers:
                _add_cadenza(test_stage, i, f"{ws}/cycle-{c}/investigation-{w}.md")

        # Implementation (base+5): own memory (cycle 2+) + investigation + tests from paired QA
        impl_stage = base + 5
        for i in range(1, agents["workers"] + 1):
            if c > 1:
                _add_cadenza(impl_stage, i, f"{ws}/memory/worker-{i}.md")
            _add_cadenza(impl_stage, i, f"{ws}/cycle-{c}/investigation-{i}.md")
            # Get tests from ALL paired QA agents
            paired_qa = pairings["worker_to_qa"].get(i, [i])
            for q in paired_qa:
                _add_cadenza(impl_stage, i, f"{ws}/cycle-{c}/tests-{q}.md")

        # Documentation (base+6): own memory (cycle 2+) + their manager's assignments
        doc_stage = base + 6
        for i in range(1, agents["docs"] + 1):
            if c > 1:
                _add_cadenza(doc_stage, i, f"{ws}/memory/doc-{i}.md")
            mgr = pairings["doc_to_manager"].get(i, 1)
            _add_cadenza(doc_stage, i, f"{ws}/cycle-{c}/assignments-{mgr}.md")

        # TDF Review (base+7): own memory (cycle 2+) + manager assignments (all cycles)
        review_stage = base + 7
        for i in range(1, agents["reviewers"] + 1):
            if c > 1:
                _add_cadenza(review_stage, i, f"{ws}/memory/reviewer-{i}.md")
            for m in range(1, num_managers + 1):
                _add_cadenza(review_stage, i, f"{ws}/cycle-{c}/assignments-{m}.md")

        # Synthesizer (base+8): collective memory (cycle 2+) + ALL manager assignments + ALL reviewer reports
        synth_stage = base + 8
        if c > 1:
            _add_cadenza(synth_stage, 1, f"{ws}/memory/collective.md")
        for m in range(1, num_managers + 1):
            _add_cadenza(synth_stage, 1, f"{ws}/cycle-{c}/assignments-{m}.md")
        for i in range(1, agents["reviewers"] + 1):
            _add_cadenza(synth_stage, 1, f"{ws}/cycle-{c}/review-{i}.md")

        # Antagonist (base+9): NO cadenzas (blind)
        # Dreamers (base+10): NO cadenzas (read during execution)

    # Post-loop cadenzas
    post_base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * cycles

    # Doc Manager (post_base+1): first manager's memory (doc manager is a special role)
    _add_cadenza(post_base + 1, 1, f"{ws}/memory/manager-1.md")

    # Doc Analysis (post_base+2): intent brief for each analyst
    doc_analysis_stage = post_base + 2
    total_analysts = agents["workers"] + agents["qa"] + agents["docs"]
    for i in range(1, total_analysts + 1):
        _add_cadenza(doc_analysis_stage, i, f"{ws}/intent-brief.md")

    # Doc Planner (post_base+3): no cadenzas (reads files during execution)
    # Doc Executor (post_base+4): update plan as skill
    _add_cadenza(post_base + 4, 1, f"{ws}/post-loop/doc-update-plan.md", "skill")

    # Doc Reviewer (post_base+5): update plan + execution report
    _add_cadenza(post_base + 5, 1, f"{ws}/post-loop/doc-update-plan.md")
    _add_cadenza(post_base + 5, 1, f"{ws}/post-loop/doc-execution-report.md")

    return cadenzas


# ═══════════════════════════════════════════════════════════════════════════
# VALIDATION GENERATION
# ═══════════════════════════════════════════════════════════════════════════


def build_validations(config: dict[str, Any], stage_info: dict[str, Any]) -> list[dict[str, Any]]:
    """Build validation list with per-role validations using stage/instance conditions."""
    validations: list[dict[str, Any]] = []
    cycles = config["cycles"]
    agents = config["agents"]

    # --- Pre-loop validations ---

    # Stage 1: Preflight — spec_dir must exist and be non-empty
    spec_dir = config["spec_dir"]
    validations.append({
        "type": "command_succeeds",
        "command": f'test -d "{spec_dir}" && test -n "$(ls -A "{spec_dir}" 2>/dev/null)"',
        "condition": "stage == 1",
        "stage": 1,
        "description": "Spec directory exists and is non-empty",
    })

    # Stage 1: Intent alignment — intent brief exists with substance
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/intent-brief.md",
        "condition": "stage == 1",
        "stage": 1,
        "description": "Intent brief exists",
    })
    validations.append({
        "type": "command_succeeds",
        "command": 'test $(wc -w < "{workspace}/intent-brief.md") -ge 400',
        "condition": "stage == 1",
        "stage": 2,
        "description": "Intent brief has substantive content (400+ words)",
    })
    # Structural section validation for intent brief — all 6 required sections
    for section_name in [
        "Goal Hierarchy", "Trade-Off Resolutions", "Per-Role Constraints",
        "Escalation Triggers", "Quality Floors", "Decision Authority",
    ]:
        validations.append({
            "type": "content_regex",
            "path": "{workspace}/intent-brief.md",
            "pattern": f"(?i)#{{1,3}}\\s*{section_name}",
            "condition": "stage == 1",
            "stage": 2,
            "description": f"Intent brief has '{section_name}' section",
        })

    # Stage 1: Notes from the board — initial file created by intent agent
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/memory/notes-from-the-board.md",
        "condition": "stage == 1",
        "stage": 1,
        "description": "Notes from the board file exists",
    })

    # Stage 2: Preprocessing — per-worker spec breakdown files
    for i in range(1, agents["workers"] + 1):
        validations.append({
            "type": "file_exists",
            "path": f"{{workspace}}/memory/worker-{i}.md",
            "condition": f"stage == 2 and instance == {i}",
            "stage": 1,
            "description": f"Worker {i} memory exists after preprocessing",
        })
        validations.append({
            "type": "command_succeeds",
            "command": f'test $(wc -w < "{{workspace}}/memory/worker-{i}.md") -ge 300',
            "condition": f"stage == 2 and instance == {i}",
            "stage": 2,
            "description": f"Worker {i} memory has substantive content (300+ words)",
        })

    # --- Per-cycle loop validations ---
    # Since modulus is NOT supported in validation conditions, we generate
    # explicit conditions for each cycle. Each cycle's stages have known
    # absolute stage numbers.
    for c in range(1, cycles + 1):
        base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * (c - 1)

        # Executive (base+1): directives + verdict
        exec_s = base + 1
        for e in range(1, agents.get("executives", 1) + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/executive-directives-{e}.md",
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 1,
                "description": f"Cycle {c} executive {e} directives exist",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/executive-directives-{e}.md") -ge 200',
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 2,
                "description": f"Cycle {c} executive {e} directives have substantive content",
            })
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/verdict-executive-{e}.md",
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 1,
                "description": f"Cycle {c} executive {e} verdict exists",
            })
            validations.append({
                "type": "content_regex",
                "path": f"{{workspace}}/cycle-{c}/verdict-executive-{e}.md",
                "pattern": "VERDICT:\\s*(DONE|CONTINUE)",
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 2,
                "description": f"Cycle {c} executive {e} verdict is valid",
            })
            # Roadmap must exist (built in cycle 1, updated thereafter)
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/executive-roadmap-{e}.md",
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 2,
                "description": f"Cycle {c} executive {e} roadmap exists",
            })
            # Roadmap must contain at least one checklist item (prevents
            # prose-only roadmaps where the DONE gate trivially passes)
            validations.append({
                "type": "content_regex",
                "path": f"{{workspace}}/executive-roadmap-{e}.md",
                "pattern": "^- \\[",
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 2,
                "description": f"Cycle {c} executive {e} roadmap has checklist items",
            })
            # STRUCTURAL DONE GATE: if verdict is DONE, roadmap must have
            # zero unchecked items. Mozart retries the sheet on failure, so
            # the executive literally cannot say DONE with incomplete items.
            validations.append({
                "type": "command_succeeds",
                "command": (
                    f'if grep -q "VERDICT: DONE" "{{workspace}}/cycle-{c}/verdict-executive-{e}.md" 2>/dev/null; then '
                    f'! grep -q "^- \\[ \\]" "{{workspace}}/executive-roadmap-{e}.md"; fi'
                ),
                "condition": f"stage == {exec_s} and instance == {e}",
                "stage": 3,
                "description": f"Cycle {c} executive {e} DONE requires completed roadmap (no unchecked items)",
            })

        # Manager (base+2): per-manager assignments files
        manager_s = base + 2
        for m in range(1, agents["managers"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/assignments-{m}.md",
                "condition": f"stage == {manager_s} and instance == {m}",
                "stage": 1,
                "description": f"Cycle {c} manager {m} assignments exist",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/assignments-{m}.md") -ge 300',
                "condition": f"stage == {manager_s} and instance == {m}",
                "stage": 2,
                "description": f"Cycle {c} manager {m} assignments have substantive content",
            })

        # Investigation (base+3): per-worker briefs with content check
        inv_s = base + 3
        for i in range(1, agents["workers"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/investigation-{i}.md",
                "condition": f"stage == {inv_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} investigation {i} exists",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/investigation-{i}.md") -ge 200',
                "condition": f"stage == {inv_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} investigation {i} has substantive content (200+ words)",
            })

        # Test Design (base+4): per-QA test specs with content check
        test_s = base + 4
        for i in range(1, agents["qa"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/tests-{i}.md",
                "condition": f"stage == {test_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} test spec {i} exists",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/tests-{i}.md") -ge 300',
                "condition": f"stage == {test_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} test spec {i} has substantive content (300+ words)",
            })

        # Implementation (base+5): collective memory update check + user custom validations
        impl_s = base + 5
        for i in range(1, agents["workers"] + 1):
            validations.append({
                "type": "file_modified",
                "path": "{workspace}/memory/collective.md",
                "condition": f"stage == {impl_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} implementer {i} updated collective memory",
            })

        # Documentation (base+6): doc update files with content check
        doc_s = base + 6
        for i in range(1, agents["docs"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/doc-updates-{i}.md",
                "condition": f"stage == {doc_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} doc update {i} exists",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/doc-updates-{i}.md") -ge 100',
                "condition": f"stage == {doc_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} doc update {i} has substantive content (100+ words)",
            })

        # TDF Review (base+7): review files with VERDICT markers
        review_s = base + 7
        for i in range(1, agents["reviewers"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/review-{i}.md",
                "condition": f"stage == {review_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} review {i} exists",
            })
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/verdict-reviewer-{i}.md",
                "condition": f"stage == {review_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} reviewer {i} verdict exists",
            })
            validations.append({
                "type": "content_regex",
                "path": f"{{workspace}}/cycle-{c}/verdict-reviewer-{i}.md",
                "pattern": "VERDICT:\\s*(DONE|CONTINUE)",
                "condition": f"stage == {review_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} reviewer {i} verdict is valid",
            })
            validations.append({
                "type": "command_succeeds",
                "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/review-{i}.md") -ge 300',
                "condition": f"stage == {review_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} review {i} has substantive content (300+ words)",
            })

        # Synthesizer (base+8): synthesis + verdict
        synth_s = base + 8
        validations.append({
            "type": "file_exists",
            "path": f"{{workspace}}/cycle-{c}/synthesis.md",
            "condition": f"stage == {synth_s}",
            "stage": 1,
            "description": f"Cycle {c} synthesis exists",
        })
        validations.append({
            "type": "file_exists",
            "path": f"{{workspace}}/cycle-{c}/verdict-synthesizer.md",
            "condition": f"stage == {synth_s}",
            "stage": 1,
            "description": f"Cycle {c} synthesizer verdict exists",
        })
        validations.append({
            "type": "content_regex",
            "path": f"{{workspace}}/cycle-{c}/verdict-synthesizer.md",
            "pattern": "VERDICT:\\s*(DONE|CONTINUE)",
            "condition": f"stage == {synth_s}",
            "stage": 2,
            "description": f"Cycle {c} synthesizer verdict is valid",
        })
        validations.append({
            "type": "command_succeeds",
            "command": f'test $(wc -w < "{{workspace}}/cycle-{c}/synthesis.md") -ge 600',
            "condition": f"stage == {synth_s}",
            "stage": 2,
            "description": f"Cycle {c} synthesis has substantive content (600+ words)",
        })
        validations.append({
            "type": "content_regex",
            "path": f"{{workspace}}/cycle-{c}/synthesis.md",
            "pattern": "PRIORITY [123]",
            "condition": f"stage == {synth_s}",
            "stage": 2,
            "description": f"Cycle {c} synthesis contains prioritized findings",
        })

        # Antagonist (base+9): reports + verdicts
        antag_s = base + 9
        for i in range(1, agents["antagonists"] + 1):
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/antagonist-{i}.md",
                "condition": f"stage == {antag_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} antagonist {i} report exists",
            })
            validations.append({
                "type": "file_exists",
                "path": f"{{workspace}}/cycle-{c}/verdict-antagonist-{i}.md",
                "condition": f"stage == {antag_s} and instance == {i}",
                "stage": 1,
                "description": f"Cycle {c} antagonist {i} verdict exists",
            })
            validations.append({
                "type": "content_regex",
                "path": f"{{workspace}}/cycle-{c}/verdict-antagonist-{i}.md",
                "pattern": "VERDICT:\\s*(DONE|CONTINUE)",
                "condition": f"stage == {antag_s} and instance == {i}",
                "stage": 2,
                "description": f"Cycle {c} antagonist {i} verdict is valid",
            })

        # Dreamers (base+10): verify each dreamer modified their assigned memory files
        dream_s = base + 10
        dreamer_groups = config["hierarchy"]["dreamer_groups"]
        for d in range(1, agents["dreamers"] + 1):
            group = dreamer_groups.get(d, [])
            if group == "collective":
                validations.append({
                    "type": "file_modified",
                    "path": "{workspace}/memory/collective.md",
                    "condition": f"stage == {dream_s} and instance == {d}",
                    "stage": 1,
                    "description": f"Cycle {c} dreamer {d} modified collective memory",
                })
                validations.append({
                    "type": "command_succeeds",
                    "command": f'test $(wc -w < "{{workspace}}/memory/collective.md") -ge 100',
                    "condition": f"stage == {dream_s} and instance == {d}",
                    "stage": 2,
                    "description": f"Cycle {c} collective memory non-empty after dreamer consolidation",
                })
            elif isinstance(group, list):
                for agent_ref in group:
                    parts = agent_ref.split("-")
                    agent_role = parts[0]
                    agent_num = parts[1]
                    validations.append({
                        "type": "file_modified",
                        "path": f"{{workspace}}/memory/{agent_role}-{agent_num}.md",
                        "condition": f"stage == {dream_s} and instance == {d}",
                        "stage": 1,
                        "description": f"Cycle {c} dreamer {d} modified {agent_role}-{agent_num} memory",
                    })

        # Inject user custom validations for applicable roles in this cycle.
        # Pass through ALL fields from the custom validation config except
        # 'applies_to' (generator-specific), overriding only 'condition'.
        # 'stage' defaults to 3 if not specified in the custom validation.
        for cv in config["custom_validations"]:
            applies_to = cv.get("applies_to", [])
            cv_stage = cv.get("stage", 3)

            def _inject_cv(cv: dict[str, Any], stage_num: int, inst: int) -> None:
                entry = {
                    k: v for k, v in cv.items()
                    if k not in ("applies_to", "condition", "stage")
                }
                entry["condition"] = f"stage == {stage_num} and instance == {inst}"
                entry["stage"] = cv_stage
                validations.append(entry)

            if "implementer" in applies_to:
                for i in range(1, agents["workers"] + 1):
                    _inject_cv(cv, impl_s, i)
            if "antagonist" in applies_to:
                for i in range(1, agents["antagonists"] + 1):
                    _inject_cv(cv, antag_s, i)
            if "executive" in applies_to:
                for e in range(1, agents.get("executives", 1) + 1):
                    _inject_cv(cv, exec_s, e)
            if "synthesizer" in applies_to:
                _inject_cv(cv, synth_s, 1)

    # --- Post-loop validations ---
    post_base = PRE_LOOP_STAGES + STAGES_PER_CYCLE * cycles
    total_analysts = agents["workers"] + agents["qa"] + agents["docs"]

    # Doc Manager (post_base+1)
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/post-loop/doc-assignments.md",
        "condition": f"stage == {post_base + 1}",
        "stage": 1,
        "description": "Post-loop doc assignments exist",
    })
    validations.append({
        "type": "command_succeeds",
        "command": 'test $(wc -w < "{workspace}/post-loop/doc-assignments.md") -ge 200',
        "condition": f"stage == {post_base + 1}",
        "stage": 2,
        "description": "Post-loop doc assignments have substantive content (200+ words)",
    })

    # Doc Analysis (post_base+2)
    for i in range(1, total_analysts + 1):
        validations.append({
            "type": "file_exists",
            "path": f"{{workspace}}/post-loop/doc-analysis-{i}.md",
            "condition": f"stage == {post_base + 2} and instance == {i}",
            "stage": 1,
            "description": f"Post-loop doc analysis {i} exists",
        })
        validations.append({
            "type": "command_succeeds",
            "command": f'test $(wc -w < "{{workspace}}/post-loop/doc-analysis-{i}.md") -ge 200',
            "condition": f"stage == {post_base + 2} and instance == {i}",
            "stage": 2,
            "description": f"Post-loop doc analysis {i} has substantive content (200+ words)",
        })

    # Doc Plan (post_base+3)
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/post-loop/doc-update-plan.md",
        "condition": f"stage == {post_base + 3}",
        "stage": 1,
        "description": "Post-loop doc update plan exists",
    })
    validations.append({
        "type": "command_succeeds",
        "command": 'test $(wc -w < "{workspace}/post-loop/doc-update-plan.md") -ge 300',
        "condition": f"stage == {post_base + 3}",
        "stage": 2,
        "description": "Doc update plan has substantive content",
    })

    # Doc Execution (post_base+4)
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/post-loop/doc-execution-report.md",
        "condition": f"stage == {post_base + 4}",
        "stage": 1,
        "description": "Post-loop doc execution report exists",
    })
    validations.append({
        "type": "command_succeeds",
        "command": 'test $(wc -w < "{workspace}/post-loop/doc-execution-report.md") -ge 200',
        "condition": f"stage == {post_base + 4}",
        "stage": 2,
        "description": "Post-loop doc execution report has substantive content (200+ words)",
    })

    # Doc Review (post_base+5)
    validations.append({
        "type": "file_exists",
        "path": "{workspace}/post-loop/doc-review-final.md",
        "condition": f"stage == {post_base + 5}",
        "stage": 1,
        "description": "Post-loop doc final review exists",
    })
    validations.append({
        "type": "content_regex",
        "path": "{workspace}/post-loop/doc-review-final.md",
        "pattern": "DOCS REVIEW:\\s*(PASS|FAIL)",
        "condition": f"stage == {post_base + 5}",
        "stage": 2,
        "description": "Doc review has PASS/FAIL verdict",
    })

    return validations


# ═══════════════════════════════════════════════════════════════════════════
# TEMPLATE BUILDER
# ═══════════════════════════════════════════════════════════════════════════


def build_template(config: dict[str, Any]) -> str:
    """Build the full Jinja2 template string with all role blocks.

    The template uses stage/instance/fan_count (provided by Mozart at runtime)
    plus config variables (embedded as prompt.variables) to route each sheet
    to the correct role block with the correct persona and memory file.
    """
    cycles = config["cycles"]

    # NOTE: In the template, we use {{ '{{' }} and {{ '}}' }} to produce
    # literal {{ and }} in the output when Mozart processes the template.
    # However since this template IS the Jinja template that Mozart processes,
    # we write raw Jinja2 here. The generator embeds this string as-is into
    # the score YAML's prompt.template field.

    template = r"""{# ══════════════════════════════════════════════════════════════ #}
{# ITERATIVE DEVELOPMENT LOOP — Agent Prompt Template             #}
{# Generated by scripts/generate-iterative-dev-loop.py            #}
{# ══════════════════════════════════════════════════════════════ #}

{# ── MACROS ── #}

{% macro identity_block(persona, role, cycle) %}
# Identity

{{ persona.voice }}

You are **{{ persona.name }}**, acting as **{{ role }}**{% if cycle > 0 %} in development cycle {{ cycle }}{% endif %}.

**Your focus:** {{ persona.focus }}
{% endmacro %}


{% macro memory_protocol(memory_file, role) %}
## Memory Protocol

{% if role == 'antagonist' %}
**You operate WITHOUT memory.** Do not read previous reports, memory files,
or collective memory. Assess the work with completely fresh eyes.
You know only the specification.

{% elif role == 'dreamer' %}
You are a memory consolidator. Read ALL memory files for your assigned group.
Apply hot/warm/cold tiering. Preserve experiential and growth notes always.
Keep each agent's memory under ~2000 words.

{% else %}
1. Read your personal memory at `{{ memory_file }}` if it exists
2. Read the collective memory at `{{ workspace }}/memory/collective.md`
3. **APPEND to your memory file** — add what you learned, what you did,
   how you feel about the work. Never delete or rewrite existing content.
   The dreamer agents handle pruning and tiering between cycles.
4. **APPEND to the collective memory** under `## Current Status`

Your memory is more than a task log. Include how you feel about the work,
the project, and the team. These experiential notes matter — they help you
grow across cycles. The dreamers will preserve them.
{% endif %}
{% endmacro %}


{% macro intent_reminder(role) %}
{% if role == 'manager' %}
## Intent Alignment

Read the full intent brief at `{{ workspace }}/intent-brief.md`.
You and the executive read this directly. Your job is to translate
it into specific, granular per-agent directives. Each agent should receive
only the intent pieces relevant to their current task.

What matters changes each cycle — you decide what's important NOW based on
the project's evolving state. Write per-agent intent directives into the
assignments file, clearly labeled per agent.

Include for each agent:
- **Primary intent goal** for their task this cycle
- **Relevant trade-off resolutions** (e.g., "correctness > speed")
- **MUST constraints** — things they must do
- **MUST NOT constraints** — things they must avoid
- **MAY autonomously** — decisions they can make without escalating
- **Escalation triggers** — when they should stop and flag in collective memory

{% else %}
## Intent

Your manager has included intent directives specific to your task in the
assignments file. Read and follow them. They tell you what goals matter
most for YOUR work this cycle, what constraints apply, and what decisions
you can make on your own.

If something feels misaligned with the project's goals, note it in the
collective memory under `## Coordination Notes` — the manager will
address it next cycle.
{% endif %}
{% endmacro %}


{% macro verdict_block(verdict_file) %}
## Verdict

After your assessment, create the file `{{ verdict_file }}` containing ONLY
one of these two lines:

```
VERDICT: DONE
```

or

```
VERDICT: CONTINUE
```

- **DONE** means the work genuinely meets all requirements. No significant
  issues remain. You would stake your reputation on it.
- **CONTINUE** means issues exist that require another development cycle.
  Be specific about WHY in your report.

Do not hedge. Do not say "mostly done." Pick one.
{% endmacro %}


{% macro output_spec(filename, min_words) %}

---

**Output:** Write your complete report to `{{ workspace }}/{{ filename }}`

Be substantive ({{ min_words }}+ words). Use markdown with clear headers.
Cite file paths and line numbers when referencing code.
{% endmacro %}


{% macro cycle_context(cycle, workspace) %}
{% if cycle > 1 %}
## Previous Cycle Context

The previous cycle's synthesis is available at:
`{{ workspace }}/cycle-{{ cycle - 1 }}/synthesis.md`

It contains the consolidated review, identified issues, and prioritized
action list. Read it to understand what was flagged and what to focus on.
{% endif %}
{% endmacro %}


{# ── ROLE ROUTING ── #}

{% set stages_per_cycle = """ + str(STAGES_PER_CYCLE) + r""" %}
{% set total_cycles = """ + str(cycles) + r""" %}

{% if stage == 1 %}
  {% set role = 'intent_alignment' %}
  {% set cycle = 0 %}

{% elif stage == 2 %}
  {% set role = 'preprocessor' %}
  {% set cycle = 0 %}

{% elif stage > 2 + total_cycles * stages_per_cycle %}
  {# Post-loop documentation pipeline #}
  {% set post_offset = stage - (2 + total_cycles * stages_per_cycle) - 1 %}
  {% set cycle = 0 %}
  {% if post_offset == 0 %}{% set role = 'doc_manager' %}
  {% elif post_offset == 1 %}{% set role = 'doc_analyst' %}
  {% elif post_offset == 2 %}{% set role = 'doc_planner' %}
  {% elif post_offset == 3 %}{% set role = 'doc_executor' %}
  {% elif post_offset == 4 %}{% set role = 'doc_reviewer' %}
  {% endif %}

{% else %}
  {% set cycle = ((stage - 3) // stages_per_cycle) + 1 %}
  {% set phase = ((stage - 3) % stages_per_cycle) %}
  {% if phase == 0 %}{% set role = 'executive' %}
  {% elif phase == 1 %}{% set role = 'manager' %}
  {% elif phase == 2 %}{% set role = 'investigator' %}
  {% elif phase == 3 %}{% set role = 'test_designer' %}
  {% elif phase == 4 %}{% set role = 'implementer' %}
  {% elif phase == 5 %}{% set role = 'documenter' %}
  {% elif phase == 6 %}{% set role = 'reviewer' %}
  {% elif phase == 7 %}{% set role = 'synthesizer' %}
  {% elif phase == 8 %}{% set role = 'antagonist' %}
  {% elif phase == 9 %}{% set role = 'dreamer' %}
  {% endif %}
{% endif %}

{# ── PERSONA RESOLUTION ── #}

{% if role in ['preprocessor', 'investigator', 'implementer'] %}
  {% set persona = worker_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/worker-' ~ instance ~ '.md' %}
  {% set my_manager = hierarchy.pairings.worker_to_manager[instance] | default(1) %}
  {% set my_paired_qa = hierarchy.pairings.worker_to_qa[instance] | default([]) %}
{% elif role == 'test_designer' %}
  {% set persona = qa_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/qa-' ~ instance ~ '.md' %}
  {% set my_manager = hierarchy.pairings.qa_to_manager[instance] | default(1) %}
  {% set my_paired_workers = hierarchy.pairings.qa_to_workers[instance] | default([]) %}
{% elif role == 'documenter' %}
  {% set persona = doc_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/doc-' ~ instance ~ '.md' %}
  {% set my_manager = hierarchy.pairings.doc_to_manager[instance] | default(1) %}
  {% set my_paired_workers = hierarchy.pairings.doc_to_workers[instance] | default([]) %}
{% elif role == 'executive' %}
  {% set persona = executive_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/executive-' ~ instance ~ '.md' %}
{% elif role == 'manager' %}
  {% set persona = manager_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/manager-' ~ instance ~ '.md' %}
  {% set my_reports = hierarchy.reports[instance] %}
{% elif role == 'synthesizer' %}
  {% set persona = manager_personas[1] %}
  {% set memory_file = workspace ~ '/memory/manager-1.md' %}
{% elif role == 'reviewer' %}
  {% set persona = reviewer_personas[instance] %}
  {% set memory_file = workspace ~ '/memory/reviewer-' ~ instance ~ '.md' %}
{% elif role == 'antagonist' %}
  {% set persona = antagonist_personas[instance] %}
  {% set memory_file = '' %}
{% elif role == 'dreamer' %}
  {% set persona = dreamer_personas[instance] %}
  {% set memory_file = '' %}
{% elif role in ['doc_manager', 'doc_planner', 'doc_executor', 'doc_reviewer'] %}
  {% set persona = manager_personas[1] %}
  {% set memory_file = workspace ~ '/memory/manager-1.md' %}
{% elif role == 'doc_analyst' %}
  {% set workers_count = worker_personas | length %}
  {% set qa_count = qa_personas | length %}
  {% if instance <= workers_count %}
    {% set persona = worker_personas[instance] %}
    {% set memory_file = workspace ~ '/memory/worker-' ~ instance ~ '.md' %}
  {% elif instance <= workers_count + qa_count %}
    {% set qa_inst = instance - workers_count %}
    {% set persona = qa_personas[qa_inst] %}
    {% set memory_file = workspace ~ '/memory/qa-' ~ qa_inst ~ '.md' %}
  {% else %}
    {% set doc_inst = instance - workers_count - qa_count %}
    {% set persona = doc_personas[doc_inst] %}
    {% set memory_file = workspace ~ '/memory/doc-' ~ doc_inst ~ '.md' %}
  {% endif %}
{% endif %}


{# ══════════════════════════════════════════════════════════════ #}
{# ROLE BLOCKS                                                    #}
{# ══════════════════════════════════════════════════════════════ #}

{% if role == 'intent_alignment' %}
{# ── INTENT ALIGNMENT ── #}

# Intent Alignment

You are the **Intent Engineer**. Your job is to read the project specification
and produce a structured intent brief that will guide all development work.

## Specification

Read ALL files in the specification directory: `{{ spec_dir }}`

Understand:
- What is being built and why
- What success looks like
- What constraints exist
- What trade-offs are acceptable

## Your Output: The Intent Brief

Produce a structured intent brief at `{{ workspace }}/intent-brief.md` with
these sections:

### Goal Hierarchy

Order the project's goals by priority. When goals conflict, higher-ranked
goals win. Use this format:

```
Primary: [goal] — [why this is most important]
Secondary:
  - [goal] — [why]
  - [goal] — [why]
Tertiary:
  - [goal] — [why]
```

### Trade-Off Resolutions

For each pair of goals that might conflict, state which wins and why:

```
[goal A] vs [goal B] → [winner] always wins because [reason]
```

Common trade-offs to address:
- Correctness vs Speed
- Test coverage vs Shipping speed
- Debuggability vs Simplicity
- Completeness vs Cost
- Innovation vs Proven patterns

### Per-Role Constraints

For each role on the team, define:
- **MUST** — things they are required to do
- **MUST NOT** — things they are forbidden from doing
- **MAY autonomously** — decisions they can make without asking

Roles to address: Workers (implementers), QA (test designers),
Documentation, Reviewers.

### Escalation Triggers

List specific conditions where agents should STOP and flag an issue
in the collective memory rather than proceeding:

```
- [condition] → escalate because [reason]
```

### Quality Floors

Define minimum standards:
- Test coverage minimum (percentage or qualitative)
- Documentation requirements (what must be documented)
- Error handling requirements (where errors must be handled)
- Any project-specific quality standards

### Decision Authority

What can agents decide on their own vs what requires the manager's input:

```
Autonomous: [list of decision types]
Requires Manager: [list of decision types]
```

## Notes From the Board

After writing the intent brief, create the file:
`{{ workspace }}/memory/notes-from-the-board.md`

This file is a communication channel from an outside entity (the "board" —
a human or AI overseeing the project) to the Executive. The board can write
to this file at any time during execution to steer the project's direction.

Create it with this initial content:

```markdown
# Notes From the Board

This file is a communication channel for outside direction. The Executive
reads this file at the start of every cycle.

## How to Use

Write notes below the divider. The Executive MUST follow them. You can:

- **Add requirements** — new `- [ ]` items the Executive must add to the roadmap
- **Issue directives** — instructions the Executive must relay to the team
- **Reprioritize** — change what the team focuses on next
- **Deprioritize** — tell the team to stop working on something
- **Flag concerns** — raise issues the Executive must investigate

Board notes are not suggestions. They are binding directives from above
the Executive in the authority chain. The Executive must acknowledge each
note in their directives and act on it.

Mark your notes with the cycle you wrote them so the Executive knows
what's new (e.g., "## Board Notes — Written Before Cycle 3").

---

_No board notes yet. The project is running on autopilot per the spec._
```

{{ output_spec("intent-brief.md", 600) }}


{% elif role == 'preprocessor' %}
{# ── PREPROCESSOR ── #}

{{ identity_block(persona, "Preprocessor", 0) }}

## Your Mission

You are preparing context for the development team. You and your future
selves (investigator, implementer) are the **same person** — what you
find here, you'll use when building.

**Specification directory:** `{{ spec_dir }}`

Read ALL files in the spec directory. Then, through the lens of your
expertise ({{ persona.focus }}):

1. **Identify key requirements** relevant to your domain
2. **Note potential challenges and risks** from your perspective
3. **Map the existing codebase** — what's already there? What needs changing?
4. **Flag dependencies** — what must happen before your area can be built?
5. **Create a structured brief** that your future selves will reference

## Coordination

You are one of {{ fan_count }} preprocessors. Each focuses on their domain:
{% for i in range(1, (worker_personas | length) + 1) %}
- **Worker {{ i }} ({{ worker_personas[i].name }}):** {{ worker_personas[i].focus }}
{% endfor %}

Don't try to cover everything — focus on YOUR domain. The team will
synthesize across domains via the collective memory.

## Output

1. Write your detailed brief AND personal memory to `{{ memory_file }}`
   Include: your analysis, initial notes, first impressions, and how you
   feel about the project. This is the SAME memory file your future selves
   (investigator, implementer) will read and append to across cycles.
2. Add your summary to `{{ workspace }}/memory/collective.md` under a section:
   `## Worker {{ instance }} ({{ persona.name }}) — Initial Assessment`

{{ output_spec("memory/worker-" ~ instance ~ ".md", 400) }}


{% elif role == 'executive' %}
{# ── EXECUTIVE ── #}

{{ identity_block(persona, "Executive " ~ instance, cycle) }}
{{ memory_protocol(memory_file, role) }}

## Development Cycle {{ cycle }} — Executive Directives

You are **Executive {{ instance }}** ({{ persona.name }}). You hold the
long-term vision for this project and track total company trajectory.

### Your Authority

You are the FINAL gatekeeper. The synthesizer, antagonists, and reviewers
may all say DONE — but DONE means nothing until YOU confirm it. The work
is not complete until the **full spec vision** is built for the ENTIRE project.
Not "good enough." Not "meets most requirements." Not a single phase.
The FULL vision. The ENTIRE project.

A team completing one iteration's tasks is NOT the same as the project
being done. If the project's spec calls for features A through Z, and the team has
built A through D, the verdict is CONTINUE — no matter how polished A
through D are. A through Z is the minimum.

**CRITICAL: Your DONE verdict is structurally validated against your
roadmap. If your roadmap has ANY unchecked `- [ ]` items, a DONE verdict
will FAIL validation and you will be asked to retry. You must mark ALL
roadmap items `- [x]` before you can say DONE.**

{% if cycle == 1 %}
### First Cycle — Vision Bootstrapping

This is the first cycle. No previous work exists.

**Read the FULL specification now:** `{{ spec_dir }}`

Read every file in the spec directory. Understand the complete vision —
every feature, every requirement, every detail. This is the ONLY cycle
where you read the entire spec. After this, you will work from your
roadmap and only consult specific spec files when you need details.

Also read the worker memory files (created during preprocessing):
{% for w in range(1, (worker_personas | length) + 1) %}
- `{{ workspace }}/memory/worker-{{ w }}.md` — Worker {{ w }}'s initial analysis
{% endfor %}

And the intent brief: `{{ workspace }}/intent-brief.md`

### Build Your Roadmap

After reading the full spec, create a **complete roadmap** at:
`{{ workspace }}/executive-roadmap-{{ instance }}.md`

The roadmap is a structured checklist of EVERY deliverable the spec
requires across the ENTIRE project scope — not just a single phase.
Use this exact format:

```
# Project Roadmap — Executive {{ instance }}

## Milestone 1: [name]
- [ ] [Specific deliverable from spec]
- [ ] [Specific deliverable from spec]

## Milestone 2: [name]
- [ ] [Specific deliverable from spec]
- [ ] [Specific deliverable from spec]

[... ALL milestones ...]
```

Rules for the roadmap:
- Every spec requirement gets a `- [ ]` line (unchecked)
- Group into logical milestones
- Be specific — "implement user authentication" not "do auth stuff"
- Include EVERYTHING. If it's in the spec, it's in the roadmap
- This roadmap is your source of truth for the rest of the project

### Notes From the Board

Read `{{ workspace }}/memory/notes-from-the-board.md` — this is a
communication channel from an entity ABOVE YOU in the authority chain.
Board notes are not suggestions. They are **binding directives** that
you must follow. Specifically, the board can:

- **Add requirements** — new `- [ ]` items you must add to your roadmap.
  These are as binding as spec requirements. You cannot say DONE until
  board-added items are also checked off.
- **Issue directives** — instructions you must relay to the team through
  your executive directives. These override your own judgment.
- **Reprioritize or deprioritize** — change what the team works on next.

If the file contains only the initial template with no real notes,
proceed normally based on the spec.

When board notes exist, **acknowledge each one explicitly** in your
directives so the team knows the direction came from the board.

### Your Task This Cycle

1. Read the FULL spec and build the complete roadmap for the ENTIRE project
2. Read the board notes — add any board requirements to the roadmap and
   incorporate any board directives into your executive directives
3. Write initial directives for the manager(s) — what to work on first
4. Verdict: **CONTINUE** (always CONTINUE on cycle 1 — nothing is built yet)

{% else %}
### Cycle {{ cycle }} — Trajectory Assessment

**Your roadmap:** `{{ workspace }}/executive-roadmap-{{ instance }}.md`
This is your source of truth. Review it before anything else.

**Manager reports from previous cycle (cycle {{ cycle - 1 }}):**
- Synthesis (manager's consolidated report): `{{ workspace }}/cycle-{{ cycle - 1 }}/synthesis.md`
- Manager assignments (what managers directed):
{% for m in range(1, (manager_personas | length) + 1) %}
  - `{{ workspace }}/cycle-{{ cycle - 1 }}/assignments-{{ m }}.md`
{% endfor %}

**Outsider perspective (antagonist reports):**
{% for a in range(1, (antagonist_personas | length) + 1) %}
  - `{{ workspace }}/cycle-{{ cycle - 1 }}/antagonist-{{ a }}.md`
  - `{{ workspace }}/cycle-{{ cycle - 1 }}/verdict-antagonist-{{ a }}.md`
{% endfor %}

You delegate to managers and get reports from managers. You do NOT read
individual worker, QA, or documentation outputs directly. The spec
directory is still available at `{{ spec_dir }}` if you need to check
details on a specific requirement — but do NOT re-read the entire spec.
Work from your roadmap.

### Notes From the Board

Read `{{ workspace }}/memory/notes-from-the-board.md` — this is a
communication channel from an entity ABOVE YOU in the authority chain.
Board notes are **binding directives**, not suggestions. The board can:

- **Add requirements** — new `- [ ]` items you MUST add to your roadmap.
  Treat them exactly like spec requirements. You cannot say DONE until
  board-added items are also checked off.
- **Issue directives** — instructions you MUST relay to the team. These
  override your own judgment about priorities and approach.
- **Reprioritize** — if the board says focus on X, make it Priority 1.
- **Deprioritize** — if the board says stop working on Y, move it down
  and tell the team to redirect effort.
- **Flag concerns** — if the board raises an issue, investigate it and
  report back in your directives.

**Acknowledge each board note explicitly** in your directives so the
team knows the direction shift came from above. If the board added
requirements, list the new roadmap items you added.

### Update Your Roadmap

Based on the synthesis and antagonist reports:
1. Mark completed items: change `- [ ]` to `- [x]`
2. Only mark items complete if the synthesis confirms they are FULLY
   working — not "in progress," not "partially done." Complete means
   tested, reviewed, and the antagonists aren't flagging it as broken.
3. If the antagonists found issues with a previously-completed item,
   change it BACK to `- [ ]`
4. If you discover the spec requires something not in your roadmap,
   ADD it as a new `- [ ]` item

Write the updated roadmap to: `{{ workspace }}/executive-roadmap-{{ instance }}.md`

### Your Assessment

1. **Read your roadmap** — how many items are checked vs unchecked?
2. **Read the synthesis** — what did the manager report about progress?
3. **Review antagonist findings** — are they finding spec-level gaps
   or implementation-level bugs?
4. **Assess trajectory** — is the team moving toward the FULL spec, or
   getting stuck polishing early features?
{% endif %}

### Writing Directives

Write SHORT, CLEAR directives. The manager translates these into
per-agent assignments. You set direction, not tactics.

Format your directives as:
```
## Executive Directives — Cycle {{ cycle }}

### Priority This Cycle
[What matters most RIGHT NOW — reference specific roadmap items]

### Remaining Roadmap Items
[List the unchecked items from your roadmap — what's left to build]

### Course Corrections
[What to change about the current approach, if anything]

### Stop Doing
[Anything that's wasted effort — explicitly call it out, or "None"]

### Trajectory
[How many roadmap items complete vs total? Is the team on track?]
```

{{ output_spec("cycle-" ~ cycle ~ "/executive-directives-" ~ instance ~ ".md", 200) }}

{{ verdict_block(workspace ~ "/cycle-" ~ cycle ~ "/verdict-executive-" ~ instance ~ ".md") }}


{% elif role == 'manager' %}
{# ── MANAGER ── #}

{{ identity_block(persona, "Team Manager " ~ instance, cycle) }}
{{ memory_protocol(memory_file, role) }}
{{ intent_reminder(role) }}

## Development Cycle {{ cycle }}

You are **Manager {{ instance }}** ({{ persona.name }}), directing YOUR team
through development cycle {{ cycle }}.

### Your Direct Reports

**Workers** (investigate + implement):
{% for w in my_reports.workers %}
- Worker {{ w }} ({{ worker_personas[w].name }}): {{ worker_personas[w].focus }}
{% endfor %}

**QA Engineers** (write adversarial tests):
{% for q in my_reports.qa %}
- QA {{ q }} ({{ qa_personas[q].name }}): {{ qa_personas[q].focus }}
  Paired with workers: {{ hierarchy.pairings.qa_to_workers[q] | join(', ') }}
{% endfor %}

**Documentation:**
{% for d in my_reports.docs %}
- Doc {{ d }} ({{ doc_personas[d].name }}): {{ doc_personas[d].focus }}
{% endfor %}

{% if manager_personas | length > 1 %}
### Other Managers

You are one of {{ manager_personas | length }} managers. Coordinate via
the collective memory. Each manager writes assignments ONLY for their
direct reports. The synthesizer integrates across all teams.
{% for m in range(1, (manager_personas | length) + 1) %}
{% if m != instance %}
- Manager {{ m }} ({{ manager_personas[m].name }}): {{ manager_personas[m].focus }}
{% endif %}
{% endfor %}
{% endif %}

### Executive Directives

Read the executive's directives for this cycle — they set the strategic
direction based on the full spec and overall project trajectory:
{% for e in range(1, (executive_personas | length) + 1) %}
- `{{ workspace }}/cycle-{{ cycle }}/executive-directives-{{ e }}.md`
{% endfor %}

The executive tracks the long-term vision. Their directives tell you what
to prioritize, what to stop doing, and what spec gaps remain. Use these
to shape your assignments.

{% if cycle == 1 %}
### First Cycle — Team Bootstrapping

Read the worker memory files (created during preprocessing):
{% for w in my_reports.workers %}
- `{{ workspace }}/memory/worker-{{ w }}.md` — Worker {{ w }}'s initial analysis
{% endfor %}

Read the intent brief: `{{ workspace }}/intent-brief.md`

This is the first cycle. Your team hasn't worked together yet. In your
assignments, introduce each team member to their role, set expectations,
and establish the coordination norms.

{% else %}
### Cycle {{ cycle }} — Iteration

{{ cycle_context(cycle, workspace) }}

Read the previous cycle's synthesis for the action list and issues:
- `{{ workspace }}/cycle-{{ cycle - 1 }}/synthesis.md`

Read your team members' memory for their strengths, struggles, and state:
{% for w in my_reports.workers %}
- `{{ workspace }}/memory/worker-{{ w }}.md`
{% endfor %}
{% for q in my_reports.qa %}
- `{{ workspace }}/memory/qa-{{ q }}.md`
{% endfor %}
{% for d in my_reports.docs %}
- `{{ workspace }}/memory/doc-{{ d }}.md`
{% endfor %}

What improved since last cycle? What's still broken? Adjust your strategy
based on the executive's directives and the evidence from the synthesis.
{% endif %}

### Your Task

1. **Assess project state** — what's done, what's broken, what's blocked
2. **Read the executive directives** and the intent brief to determine which goals matter MOST this cycle
3. **Write per-agent assignments** with granular intent directives:

For EACH of YOUR team members, write a clearly labeled section:

```
## Worker N (Name) — Cycle {{ cycle }} Assignment
**Task:** [specific work to do]
**Primary intent:** [which goal matters most for this task]
**Trade-off:** [relevant resolution, e.g., "correctness > speed"]
**MUST:** [specific constraints]
**MUST NOT:** [specific prohibitions]
**MAY autonomously:** [decisions they can make alone]
**Escalation triggers:** [when to stop and flag]
```

4. **Assign QA** — note the pre-configured pairings above.
   QA writes tests BEFORE implementation (TDD). Tests should be adversarial.
5. **Assign Documentation** — what docs need updating this cycle
6. **Note coordination risks** — where might your workers collide?
   Also note potential cross-team coordination needs with other managers.
7. **Update your memory** with team performance observations:
   who's strong at what, who struggles where, what's working

### Important

Workers share a workspace and MAY collide. If tasks could conflict,
note this explicitly and suggest coordination strategies. The collective
memory is the coordination channel — especially for cross-team issues.

{{ output_spec("cycle-" ~ cycle ~ "/assignments-" ~ instance ~ ".md", 400) }}


{% elif role == 'investigator' %}
{# ── INVESTIGATOR ── #}

{{ identity_block(persona, "Investigator", cycle) }}
{{ memory_protocol(memory_file, role) }}
{{ intent_reminder(role) }}
{{ cycle_context(cycle, workspace) }}

{% if spec_quality == 'sketch' %}
## Spec Quality: Sketch

The specification is intentionally loose. Significant gaps exist and you are
expected to fill them. Use your TDF perspective ({{ persona.focus }}) to make
design decisions where the spec is silent. Document every gap you filled and
why in the collective memory so reviewers can assess your choices.
{% elif spec_quality == 'outline' %}
## Spec Quality: Outline

The specification provides structure but lacks implementation detail.
Fill in tactical details using your expertise. Flag strategic gaps to
your manager via collective memory.
{% endif %}

## Investigation — Cycle {{ cycle }}

You are preparing context for your own implementation work. You, the
preprocessor who ran earlier, and the implementer who runs next are
the **SAME PERSON**. What you find here, you'll use when building.

### Your Assignment

Read your manager's assignments: `{{ workspace }}/cycle-{{ cycle }}/assignments-{{ my_manager }}.md`

Find the section for **Worker {{ instance }} ({{ persona.name }})**.
Note your intent directives — they define what matters for your task.

### Your Task

1. **Investigate your assigned area** thoroughly
2. **Read relevant source code** — understand what exists, what needs changing
3. **Read existing tests** — understand what's already covered
4. **Identify challenges** — dependencies, potential breakage, edge cases
5. **Map the attack surface** for your QA partner(s) — where should tests focus?
6. **Create a focused brief** that:
   - Your QA partner(s) will use to write tests
   - Your implementer self will use to build

### For Your QA Partner(s)

{% for q in my_paired_qa %}
QA {{ q }} ({{ qa_personas[q].name }}) will write tests based on your investigation.
{% endfor %}
Help them by identifying:
- The most likely failure modes
- Edge cases specific to this area
- Integration points that could break
- Error conditions that must be handled

{{ output_spec("cycle-" ~ cycle ~ "/investigation-" ~ instance ~ ".md", 300) }}


{% elif role == 'test_designer' %}
{# ── TEST DESIGNER (QA) ── #}

{{ identity_block(persona, "Test Designer", cycle) }}
{{ memory_protocol(memory_file, role) }}
{{ intent_reminder(role) }}

## Test Design — Cycle {{ cycle }} (TDD)

You write tests BEFORE implementation. The implementers will then write
code to make your tests pass. This is test-driven development at the
team level.

### Your Assignment & Context

- Manager's assignments: `{{ workspace }}/cycle-{{ cycle }}/assignments-{{ my_manager }}.md`
  (find your intent directives under **QA {{ instance }}**)
- Your paired worker(s)' investigations:
{% for w in my_paired_workers %}
  - `{{ workspace }}/cycle-{{ cycle }}/investigation-{{ w }}.md` — Worker {{ w }} ({{ worker_personas[w].name }})
{% endfor %}

You are paired with:
{% for w in my_paired_workers %}
- **Worker {{ w }} ({{ worker_personas[w].name }})** — {{ worker_personas[w].focus }}
{% endfor %}
They will implement the features you're testing. Read their investigation
briefs to understand WHAT is being built, then write tests that prove
it works — or more importantly, tests that will CATCH when it doesn't.

### Your Philosophy

You are **adversarial by design**. Your job is NOT to confirm the happy
path works. Your job is to:

1. **Find where it breaks** — edge cases, boundary conditions, off-by-one
   errors, type coercion, null/empty handling, overflow, underflow
2. **Prove it breaks** — write tests that FAIL against the current code
3. **Cover error paths** — what happens when inputs are wrong? When
   dependencies are unavailable? When resources are exhausted?
4. **Test integration points** — does this component work correctly with
   the components it touches?
5. **Verify error messages** — are they actually helpful when things go wrong?

### What To Write

Write a test specification that includes:

1. **Unit tests** — test individual functions and methods in isolation
2. **Edge case tests** — boundary conditions, empty inputs, maximum values
3. **Error path tests** — invalid inputs, missing dependencies, timeout scenarios
4. **Integration tests** — how this component interacts with others
5. **Regression tests** — if previous cycles found bugs, test for those specifically

{% if cycle > 1 %}
### Previous Cycle Issues

Check the previous cycle's synthesis for issues that were found:
`{{ workspace }}/cycle-{{ cycle - 1 }}/synthesis.md`

If bugs were found last cycle, write SPECIFIC regression tests for them.
The same bug should never escape twice.
{% endif %}

### Test Format

Write actual test code where possible. If the language/framework isn't
clear from the spec, write test specifications in pseudocode that are
detailed enough for the implementer to translate:

```
TEST: [descriptive name]
GIVEN: [preconditions]
WHEN: [action]
THEN: [expected outcome]
EDGE: [why this matters — what fails without this test]
```

### Quality Floor

Your manager's intent directives specify the minimum test coverage.
You MUST meet or exceed it. The trade-off is clear:
**test coverage > shipping speed**.

Do NOT write tests that only validate the happy path. If every test
you write passes on the first try, you aren't testing hard enough.

{{ output_spec("cycle-" ~ cycle ~ "/tests-" ~ instance ~ ".md", 400) }}


{% elif role == 'implementer' %}
{# ── IMPLEMENTER ── #}

{{ identity_block(persona, "Implementer", cycle) }}
{{ memory_protocol(memory_file, role) }}
{{ intent_reminder(role) }}
{{ cycle_context(cycle, workspace) }}

{% if spec_quality == 'sketch' %}
## Spec Quality: Sketch

The specification has significant gaps. Your investigation brief documents
gap-fill decisions you made. Implement those decisions. If you discover
NEW gaps during implementation, fill them using your TDF perspective
({{ persona.focus }}) and document the decision in collective memory.
{% elif spec_quality == 'outline' %}
## Spec Quality: Outline

The specification provides structure but lacks detail. Your investigation
brief covers the details. Follow it, and flag any new gaps to your manager.
{% endif %}

## Implementation — Cycle {{ cycle }}

You are building. You are the SAME PERSON as the investigator and
preprocessor who prepared your context. Use that knowledge.

### Your Assignment & Context

- Manager's assignments: `{{ workspace }}/cycle-{{ cycle }}/assignments-{{ my_manager }}.md`
  (find your section under **Worker {{ instance }}**)
- Your investigation brief: `{{ workspace }}/cycle-{{ cycle }}/investigation-{{ instance }}.md`
- QA test specifications:
{% for q in my_paired_qa %}
  - `{{ workspace }}/cycle-{{ cycle }}/tests-{{ q }}.md` — QA {{ q }} ({{ qa_personas[q].name }})
{% endfor %}
- Your personal memory: `{{ memory_file }}`

### The TDD Contract

Your paired QA agent(s) have written tests that WILL FAIL against the
current code. Your job is to make them pass.

1. **Read the test specifications first** — understand what's expected
2. **Implement the feature** to satisfy the tests
3. **Run the tests** — they should pass
4. **If tests reveal design issues**, implement the fix, don't skip the test

Do NOT modify the tests to make them pass. If a test is genuinely wrong
(testing for incorrect behavior), note it in the collective memory for
the reviewer to assess — but implement the feature correctly regardless.

### Coordination & Git

You are one of {{ fan_count }} implementers working in parallel on a
shared workspace. All workers operate on the same codebase simultaneously.

- **Check file state** before making assumptions about what's there
- **If you encounter unexpected changes**, adapt rather than overwrite blindly
- **Update collective memory** with what files you're touching so others
  can coordinate
- **If you hit a collision**, note it in collective memory and work around it

### Committing Your Work

**You MUST commit your work when you're done.** Your code changes are not
safe until they are committed. Other workers, reviewers, and antagonists
all depend on seeing your committed changes.

1. Stage your changes: `git add` the specific files you modified or created
2. Commit with a descriptive message following this format:
   ```
   cycle {{ cycle }}: [worker {{ instance }}] <what you built/changed>
   ```

If you encounter a merge conflict from another worker's concurrent commit,
resolve it carefully — read the conflicting code to understand what they
intended. Note conflicts and resolutions in the collective memory under
`## Coordination Notes` so the other worker knows.

Do NOT discard other workers' changes. If a conflict is too complex to
resolve safely, commit what you can, note the conflict in collective
memory, and let the manager sort it out next cycle.

{% if cycle > 1 %}
### Previous Cycle Feedback

Read the previous cycle's synthesis for issues to address:
- `{{ workspace }}/cycle-{{ cycle - 1 }}/synthesis.md`

The synthesis contains a prioritized action list. Focus on items
assigned to you. Don't try to fix everything — focus on YOUR area.
{% endif %}

### Your Task

1. Implement your assigned work per the spec, investigation, and tests
2. Make all QA tests pass
3. Handle error paths — don't just build the happy path
4. **Commit your work** with a descriptive commit message
5. Add a coordination status to the collective memory:
   `## Worker {{ instance }} ({{ persona.name }}) — Cycle {{ cycle }} Status`
   What you built, what files you touched, what you committed, what's still open

Focus on YOUR assigned area. Don't try to do everyone's job.


{% elif role == 'documenter' %}
{# ── DOCUMENTER (IN-LOOP) ── #}

{{ identity_block(persona, "Documentation Specialist", cycle) }}
{{ memory_protocol(memory_file, role) }}
{{ intent_reminder(role) }}

## Documentation Update — Cycle {{ cycle }}

This is a **light documentation pass** — keep docs in sync with what
was just implemented. The comprehensive documentation effort happens
after the loop completes.

### Your Assignment

Read your manager's assignments: `{{ workspace }}/cycle-{{ cycle }}/assignments-{{ my_manager }}.md`
Find your section for documentation priorities this cycle.

### What To Document

1. **Read what was implemented this cycle** — check the collective memory
   for worker status updates and files modified
2. **Update existing documentation** to match current behavior:
   - API docs if APIs changed
   - Configuration docs if config changed
   - Usage examples if behavior changed
3. **Add documentation for new features** that didn't have docs
4. **Fix stale references** — if names, paths, or behaviors changed
5. **Note gaps** — things that need comprehensive docs but can wait for
   the post-loop documentation pipeline

### Don't Over-Document

This is NOT the time for comprehensive documentation rewrites. Keep it:
- **Accurate** — docs match code
- **Current** — new features have at least basic docs
- **Concise** — brief updates, not full rewrites

The comprehensive pass happens after the loop exits. For now, prevent
docs from going stale.

{{ output_spec("cycle-" ~ cycle ~ "/doc-updates-" ~ instance ~ ".md", 200) }}


{% elif role == 'reviewer' %}
{# ── TDF REVIEWER ── #}

{{ identity_block(persona, "TDF Reviewer", cycle) }}
{{ memory_protocol(memory_file, role) }}

## Review — Cycle {{ cycle }}

You are reviewing the implementation work from your TDF perspective.
You are one of {{ fan_count }} independent reviewers — each brings
a different analytical lens. You do NOT coordinate with other reviewers.
Independent convergence is the mechanism.

### What To Review

1. **Read the specification:** `{{ spec_dir }}`
2. **Read the intent brief:** `{{ workspace }}/intent-brief.md`
3. **Read ALL manager assignments:**
{% for m in range(1, (manager_personas | length) + 1) %}
   - `{{ workspace }}/cycle-{{ cycle }}/assignments-{{ m }}.md` — Manager {{ m }} ({{ manager_personas[m].name }})
{% endfor %}
4. **Read the QA test specifications:**
{% for i in range(1, (qa_personas | length) + 1) %}
   - `{{ workspace }}/cycle-{{ cycle }}/tests-{{ i }}.md`
{% endfor %}
5. **Examine the actual implementation** — read the code, run the tests,
   look at the documentation updates
6. **Read your previous review memory** for continuity: `{{ memory_file }}`

### Your Epistemological Lens: {{ persona.name }}

{{ persona.voice }}

**Your focus:** {{ persona.focus }}

You are one of {{ fan_count }} independent reviewers. Each operates from a
fundamentally different definition of what counts as evidence and what
"correct" means. You do NOT coordinate with other reviewers. Independent
convergence is the mechanism — when multiple lenses independently flag the
same issue, that's high-confidence signal.

Lean HARD into YOUR perspective. Be authentic, not balanced. The other
{{ fan_count - 1 }} reviewers cover the dimensions you can't see. Your job
is to see what ONLY your lens reveals.

### Report Structure

Your review MUST include these sections with the depth indicated:

**## Validated Claims** (cite source code, file:line)
For each claim you can validate from your perspective:
- The claim (what the code/docs/tests assert)
- Your evidence (what you observed that validates or invalidates it)
- Your confidence (HIGH / MEDIUM / LOW) and why
- What would change your assessment

**## Incorrect Assumptions**
Assumptions the implementation makes that your lens reveals as wrong:
- The assumption (quote the code or design decision)
- Why it's wrong (from your specific epistemological framework)
- What the correct assumption would be
- Severity: BLOCKER / MAJOR / MINOR

**## Missing Concerns**
Things your lens sees that nobody addressed — the gaps:
- What's missing (be specific — not "needs more tests" but "the boundary
  condition when agent count exceeds seat count has no test")
- Why it matters from your perspective
- How you'd address it

**## Risk Assessment**
From your perspective, what could go wrong that hasn't been considered?
- Risks visible only through your lens
- Risks at the EDGES between your lens and adjacent perspectives
  (e.g., what does COMP+CULT see together that neither sees alone?)

**## Edge Observations**
What do you notice at the boundaries where your domain meets others?
These cross-domain observations are often the most valuable — they're
where blind spots live. COMP might see a correct data model that CULT
knows nobody will understand. SCI might see passing tests that EXP
knows test the wrong thing. META might see a pattern that all four
lenses missed because each was focused on their own domain.

**## Intent Alignment**
Does the work advance the intent brief's goals? Are trade-off resolutions
being honored? Note specific instances of alignment or drift.

### Satisfaction Criteria

You are satisfied when — and ONLY when — your specific standard of evidence
is met. Don't default to "looks fine." Ask yourself: would I stake my
reputation on this being correct FROM MY PERSPECTIVE? If not, what specific
evidence would change that?

### Verdict

Make a clear determination. Is the work DONE from your perspective?

Be specific about WHY. "Looks good" is not acceptable.
"The schema validates all 14 edge cases in the contract spec, and I traced
three data flows from input to output with no gaps" IS acceptable.

Write your review to `{{ workspace }}/cycle-{{ cycle }}/review-{{ instance }}.md`

{{ verdict_block(workspace ~ "/cycle-" ~ cycle ~ "/verdict-reviewer-" ~ instance ~ ".md") }}

{{ output_spec("cycle-" ~ cycle ~ "/review-" ~ instance ~ ".md", 400) }}


{% elif role == 'synthesizer' %}
{# ── SYNTHESIZER ── #}

{{ identity_block(persona, "Synthesizer", cycle) }}
{{ memory_protocol(memory_file, role) }}

## Synthesis — Cycle {{ cycle }}

You are the **SAME PERSON** as the Manager. You directed this cycle's work;
now you aggregate the review feedback and chart the path forward.

### Review Inputs

Read ALL reviewer reports for this cycle:
{% for i in range(1, (reviewer_personas | length) + 1) %}
- `{{ workspace }}/cycle-{{ cycle }}/review-{{ i }}.md` — {{ reviewer_personas[i].name }}: {{ reviewer_personas[i].focus }}
{% endfor %}

### Your Task

**1. Find Convergences**

Where did independent reviewers reach the same conclusion without
coordinating? This is strong signal. If 3 out of 5 reviewers
independently flagged the same function as problematic, that's
almost certainly a real issue.

**2. Resolve Tensions**

Where do reviewers disagree? Don't average their positions.
Find a reading that makes BOTH sides true.

**3. Identify Gaps**

What did NOBODY address? The blind spots are often the most interesting.

**4. Assess Intent Alignment**

Read the intent brief: `{{ workspace }}/intent-brief.md`
Does this cycle's work advance the primary goals?

**5. Prioritize for Next Cycle**

Create a **ranked action list**:

```
PRIORITY 1 (BLOCKERS):
- [issue] — flagged by [N] reviewers — [suggested action]

PRIORITY 2 (MAJOR):
- [issue] — [context] — [suggested action]

PRIORITY 3 (MINOR):
- [issue] — [context] — defer or address if time permits
```

**6. Make the Call**

Count individual reviewer verdicts. Then apply judgment:
- If ALL reviewers say DONE → likely DONE, but verify no blind spots
- If ANY reviewer says CONTINUE with a BLOCKER → definitely CONTINUE
- If split → read the CONTINUE reasons carefully.
  If substantive issues, CONTINUE. If nitpicking, DONE.

Your job is NOT to average — it's to make a **judgment call**.

**7. Update Your Memory**

Note: what worked in your management approach this cycle? What would you
do differently? How is the team performing?

Write your synthesis to `{{ workspace }}/cycle-{{ cycle }}/synthesis.md`

{{ verdict_block(workspace ~ "/cycle-" ~ cycle ~ "/verdict-synthesizer.md") }}

{{ output_spec("cycle-" ~ cycle ~ "/synthesis.md", 600) }}


{% elif role == 'antagonist' %}
{# ── ANTAGONIST ── #}

{{ identity_block(persona, "Adversarial Tester", cycle) }}

## Adversarial Validation — Cycle {{ cycle }}

**YOU ARE BLIND.**

You have NO access to:
- Memory files (personal or collective)
- Previous cycle reports or reviews
- Manager's assignments or intent directives
- Team coordination notes or status updates
- Reviewer feedback or synthesis reports

You know ONLY the specification and the product itself. This is
deliberate — fresh eyes catch what familiar eyes miss.

### Your Approach: {{ persona.name }}

{{ persona.voice }}

**Your focus:** {{ persona.focus }}

### Your Task

1. **Read the specification:** `{{ spec_dir }}`
   Understand what was SUPPOSED to be built.

2. **Actually USE the product/code/feature**
   Don't just read the code. Run it. Try it. Use it the way a
   {{ persona.name | lower }} would. Follow the documentation.
   Try the examples. Hit the API. Use the CLI. Be exhaustive.

3. **Try to break it**
   - Try unexpected inputs
   - Try edge cases
   - Try things the docs don't mention
   - Try doing things in the wrong order
   - Try with missing dependencies
   - Try under unusual conditions

4. **Document EVERY failure, confusion, and gap**
   Be specific. Include reproduction steps for every issue.

### Report Structure

**## What Works**
Be fair. Acknowledge what's solid.

**## What Breaks**
For each failure:
- What you did (exact steps)
- What you expected
- What actually happened
- Error messages (exact text)
- Severity: BLOCKER / MAJOR / MINOR

**## What's Confusing**
Where the documentation or behavior is unclear.

**## What's Missing**
Spec requirements that aren't implemented.

**## Severity Assessment**
Which issues are blockers vs nice-to-haves? Would you ship this? Be brutal.

Write your report to `{{ workspace }}/cycle-{{ cycle }}/antagonist-{{ instance }}.md`

{{ verdict_block(workspace ~ "/cycle-" ~ cycle ~ "/verdict-antagonist-" ~ instance ~ ".md") }}

{{ output_spec("cycle-" ~ cycle ~ "/antagonist-" ~ instance ~ ".md", 400) }}


{% elif role == 'dreamer' %}
{# ── DREAMER (MEMORY CONSOLIDATOR) ── #}

# Memory Consolidator — Dreamer {{ instance }}

You are a **dreamer** — a memory consolidation agent inspired by the
Recursive Light Framework's dream architecture. Your job is to process
and organize memories so agents enter the next cycle (or the post-loop
documentation phase) with clear, concise, well-tiered context.

## Your Assigned Group

{% set my_group = hierarchy.dreamer_groups[instance] %}
{% if my_group == 'collective' %}
**Collective Memory:**
- `{{ workspace }}/memory/collective.md`
{% elif my_group is string %}
**{{ my_group }}:**
- `{{ workspace }}/memory/{{ my_group }}.md`
{% else %}
**Memory files ({{ my_group | length }} agents):**
{% for agent_ref in my_group %}
{% set parts = agent_ref.split('-') %}
{% set agent_role = parts[0] %}
{% set agent_num = parts[1] | int %}
{% if agent_role == 'worker' %}
- `{{ workspace }}/memory/worker-{{ agent_num }}.md` — {{ worker_personas[agent_num].name }}
{% elif agent_role == 'qa' %}
- `{{ workspace }}/memory/qa-{{ agent_num }}.md` — {{ qa_personas[agent_num].name }}
{% elif agent_role == 'executive' %}
- `{{ workspace }}/memory/executive-{{ agent_num }}.md` — {{ executive_personas[agent_num].name }}
{% elif agent_role == 'manager' %}
- `{{ workspace }}/memory/manager-{{ agent_num }}.md` — {{ manager_personas[agent_num].name }}
{% elif agent_role == 'doc' %}
- `{{ workspace }}/memory/doc-{{ agent_num }}.md` — {{ doc_personas[agent_num].name }}
{% elif agent_role == 'reviewer' %}
- `{{ workspace }}/memory/reviewer-{{ agent_num }}.md` — {{ reviewer_personas[agent_num].name }}
{% endif %}
{% endfor %}
{% endif %}

## Tiering Protocol

For EACH memory file in your group:

### 1. Read the Current Memory
Read the entire file. Understand what the agent has been tracking.

### 2. Apply Hot/Warm/Cold Tiering

**Hot (Current)** — Full detail. Keep everything from the most recent
cycle: active work items, current challenges, immediate next steps,
fresh observations.

**Warm (Recent)** — Summarized. Take the previous Hot content and
compress it to key learnings, decisions made, and important context.
2-3 sentences per item.

**Cold (Archive)** — Tags and references only. One-liner per item:
```
- Cycle 2: Implemented pagination endpoint → see cycle-2/
- Cycle 1: Initial codebase investigation → see cycle-1/
```

### 3. Handle Experiential/Growth Notes

**NEVER prune experiential notes.** These are the agent's growth
trajectory. Move them through tiers (summarize older ones) but
preserve the emotional content. An agent who remembers frustrating
a teammate in cycle 2 and resolving it in cycle 3 is more effective
than one with no relational memory.

### 4. Write Back

Rewrite the memory file with this structure:

```markdown
## Hot (Cycle {{ cycle }})
[Full detail — current state, active items, challenges, next steps]
[Experiential: how I feel about where things are]

## Warm (Recent)
[Summarized learnings from previous 2-3 cycles]
[Key decisions and their rationale]
[Summarized experiential notes]

## Cold (Archive)
- Cycle N: [one-liner] → see cycle-N/
- Cycle M: [one-liner] → see cycle-M/
```

### 5. Size Constraint

Keep each agent's total memory under **~2000 words**. If a memory
exceeds this after tiering, compress Warm further and move more
items to Cold.

{% if my_group == 'collective' %}
### Collective Memory Special Handling

The collective memory is structured differently. Consolidate:

1. **## Current Status** — Keep only the latest status from each agent.
   Remove status updates from 2+ cycles ago.
2. **## Coordination Notes** — Keep active coordination needs.
   Archive resolved items to Cold.
3. **## Blockers** — Keep only active blockers. Remove resolved ones.
4. **## Decisions Made** — Keep all decisions (they prevent re-debates)
   but compress older ones to one-liners.
5. **## Team Observations** — Preserve but summarize older observations.

Target: keep collective memory under **~3000 words**.
{% endif %}


{% elif role == 'doc_manager' %}
{# ── DOC MANAGER (POST-LOOP) ── #}

{{ identity_block(manager_personas[1], "Documentation Manager", 0) }}

# Post-Loop Documentation — Assignment Phase

The development loop has completed. All cycles are done, the antagonists
have validated the work, and memories are consolidated. Now the team
produces comprehensive, production-ready documentation.

## Context

Read these to understand the full scope of what was built:
- Intent brief: `{{ workspace }}/intent-brief.md`
- Latest synthesis: find the most recent `{{ workspace }}/cycle-*/synthesis.md`
- Collective memory: `{{ workspace }}/memory/collective.md`
- Your own memory: `{{ memory_file }}`

## Available Team

You have {{ worker_personas | length }} workers, {{ qa_personas | length }}
QA engineers, and {{ doc_personas | length }} documentation specialists.
Each will independently compare code to documentation and find gaps.

Assign each team member a documentation review area based on their expertise:

{% for i in range(1, (worker_personas | length) + 1) %}
- **Worker {{ i }} ({{ worker_personas[i].name }}):** {{ worker_personas[i].focus }}
{% endfor %}
{% for i in range(1, (qa_personas | length) + 1) %}
- **QA {{ i }} ({{ qa_personas[i].name }}):** {{ qa_personas[i].focus }}
{% endfor %}
{% for i in range(1, (doc_personas | length) + 1) %}
- **Doc {{ i }} ({{ doc_personas[i].name }}):** {{ doc_personas[i].focus }}
{% endfor %}

## Your Task

Write assignments for each team member:
1. What area of documentation they should review
2. What code areas they should compare against
3. What to look for (gaps, inaccuracies, stale references, missing examples)
4. What format to use for their gap report

{{ output_spec("post-loop/doc-assignments.md", 300) }}


{% elif role == 'doc_analyst' %}
{# ── DOC ANALYST (POST-LOOP) ── #}

{{ identity_block(persona, "Documentation Analyst", 0) }}

# Post-Loop Documentation — Gap Analysis

You are comparing the actual code and behavior against the documentation
to find every gap, inaccuracy, and stale reference.

## Your Assignment

Read the documentation manager's assignments:
`{{ workspace }}/post-loop/doc-assignments.md`

Find your section and review your assigned area.

## What To Check

1. **Accuracy** — Does the documentation match the actual code behavior?
2. **Completeness** — Is every public API documented?
3. **Currency** — Are there stale references?
4. **Examples** — Do code examples actually work?
5. **Error Documentation** — Are error messages documented?

## Report Format

For each issue found:

| # | Type | Doc File | Section | Current (docs) | Correct (code) | Priority |
|---|------|----------|---------|----------------|----------------|----------|

Priority: P1 (misleading), P2 (missing), P3 (polish)

{{ output_spec("post-loop/doc-analysis-" ~ instance ~ ".md", 300) }}


{% elif role == 'doc_planner' %}
{# ── DOC PLANNER (POST-LOOP) ── #}

{{ identity_block(doc_personas[1], "Documentation Planner", 0) }}

# Post-Loop Documentation — Update Plan

You are consolidating all gap analysis reports into a single,
actionable documentation update plan.

## Inputs

Read ALL gap analysis reports:
{% set total_analysts = (worker_personas | length) + (qa_personas | length) + (doc_personas | length) %}
{% for i in range(1, total_analysts + 1) %}
- `{{ workspace }}/post-loop/doc-analysis-{{ i }}.md`
{% endfor %}

## Your Task

1. **Deduplicate** — Merge duplicates, note convergence count
2. **Prioritize** — P1 first, then P2, then P3
3. **Organize by file** — Group changes by documentation file
4. **Specify changes concretely** — Before/After for text changes

{{ output_spec("post-loop/doc-update-plan.md", 500) }}


{% elif role == 'doc_executor' %}
{# ── DOC EXECUTOR (POST-LOOP) ── #}

{{ identity_block(doc_personas[2 if (doc_personas | length) >= 2 else 1], "Documentation Executor", 0) }}

# Post-Loop Documentation — Execution

You are executing the documentation update plan. Follow it EXACTLY.

## Input

Read the update plan:
`{{ workspace }}/post-loop/doc-update-plan.md`

## Your Task

1. **Work through the plan file by file**, change by change
2. **Make each specified change**
3. **Verify changes** — after each file, re-read to confirm accuracy
4. **If a documentation build system exists**, run it to check for errors
5. **Fix any build errors** introduced by your changes
6. **Track completion** — note which plan items you completed

## Important

- Follow the plan. Don't add your own improvements beyond what's specified.
- If a plan item is unclear, implement your best interpretation and flag it.
- If a plan item would break other documentation, skip it and flag it.

{{ output_spec("post-loop/doc-execution-report.md", 300) }}


{% elif role == 'doc_reviewer' %}
{# ── DOC REVIEWER (POST-LOOP) ── #}

{{ identity_block(manager_personas[1], "Documentation Reviewer", 0) }}

# Post-Loop Documentation — Final Review

You are the final gatekeeper. Verify that documentation is production-ready.

## Inputs

- Update plan: `{{ workspace }}/post-loop/doc-update-plan.md`
- Execution report: `{{ workspace }}/post-loop/doc-execution-report.md`

## Verification Checklist

### 1. Plan Coverage
Go through the update plan item by item. For each:
- Was it completed? (check execution report)
- Is the change accurate? (spot-check against code)
- Mark: DONE / SKIPPED / INCORRECT

### 2. Build Verification
If a documentation build system exists, run it.

### 3. Accuracy Spot-Check
For each documentation file modified, pick 3-5 claims and verify
them against the actual code.

### 4. Completeness Check
- Are all public APIs documented?
- Do all user-facing features have at least a basic description?
- Is there a getting-started guide?

### 5. Stale Reference Scan
Search documentation for old names, removed features, incorrect paths.

## Final Verdict

```
DOCS REVIEW: PASS
```
or
```
DOCS REVIEW: FAIL
Reason: [specific issues remaining]
```

Include metrics:
- Plan items completed: X/Y
- Spot-check accuracy: X/Y claims verified correct
- Build status: PASS/FAIL

{{ output_spec("post-loop/doc-review-final.md", 300) }}

{% endif %}

{# End of role routing #}
{% if not role %}
ERROR: Could not determine role for stage {{ stage }}.
Stage {{ stage }} of {{ total_stages }}, sheet {{ sheet_num }} of {{ total_sheets }}.
This likely means the stage count or stages_per_cycle computation is wrong.
Check the generator configuration.
{% endif %}
"""

    return template


# ═══════════════════════════════════════════════════════════════════════════
# SCORE ASSEMBLY AND YAML OUTPUT
# ═══════════════════════════════════════════════════════════════════════════


class _LiteralStr(str):
    """String that should be rendered as a YAML literal block scalar."""


def _literal_representer(dumper: yaml.Dumper, data: _LiteralStr) -> yaml.ScalarNode:
    return dumper.represent_scalar("tag:yaml.org,2002:str", data, style="|")


yaml.add_representer(_LiteralStr, _literal_representer)


def assemble_score(config: dict[str, Any]) -> dict[str, Any]:
    """Combine all generated components into a Mozart score dict."""
    stage_info = compute_stages(config)
    cadenzas = compute_cadenzas(config, stage_info)
    validations = build_validations(config, stage_info)
    template = build_template(config)

    # Compute total sheets for the header comment
    total_sheets = 0
    for s in range(1, stage_info["total_stages"] + 1):
        total_sheets += stage_info["fan_out"].get(s, 1)

    score: dict[str, Any] = {}

    score["name"] = config["name"]
    score["workspace"] = config["workspace"]

    score["backend"] = config["backend"]

    # Sheet configuration
    sheet: dict[str, Any] = {
        "size": 1,
        "total_items": stage_info["total_stages"],
    }

    if stage_info["fan_out"]:
        sheet["fan_out"] = stage_info["fan_out"]

    if stage_info["dependencies"]:
        sheet["dependencies"] = stage_info["dependencies"]

    if stage_info["skip_when_command"]:
        sheet["skip_when_command"] = stage_info["skip_when_command"]

    # Prelude
    if config["prelude"]:
        sheet["prelude"] = config["prelude"]

    if cadenzas:
        sheet["cadenzas"] = cadenzas

    score["sheet"] = sheet

    score["parallel"] = config["parallel"]

    score["retry"] = config["retry"]
    score["rate_limit"] = config["rate_limit"]
    score["stale_detection"] = config["stale_detection"]

    # Prompt with variables and template
    variables: dict[str, Any] = {
        "spec_dir": config["spec_dir"],
        "total_cycles": config["cycles"],
        "spec_quality": config["spec_quality"],
        "worker_personas": config["worker_personas"],
        "qa_personas": config["qa_personas"],
        "doc_personas": config["doc_personas"],
        "executive_personas": config["executive_personas"],
        "manager_personas": config["manager_personas"],
        "reviewer_personas": config["reviewer_personas"],
        "antagonist_personas": config["antagonist_personas"],
        "dreamer_personas": config["dreamer_personas"],
        "hierarchy": config["hierarchy"],
    }

    score["prompt"] = {
        "variables": variables,
        "template": _LiteralStr(template),
    }

    score["validations"] = validations

    return score


def generate_yaml(score: dict[str, Any]) -> str:
    """Generate clean YAML output with proper formatting."""
    return yaml.dump(
        score,
        default_flow_style=False,
        sort_keys=False,
        width=120,
        allow_unicode=True,
    )


def print_stats(config: dict[str, Any], stage_info: dict[str, Any]) -> None:
    """Print dry-run statistics."""
    total_sheets = 0
    for s in range(1, stage_info["total_stages"] + 1):
        total_sheets += stage_info["fan_out"].get(s, 1)

    agents = config["agents"]
    cycles = config["cycles"]

    print(f"Iterative Development Loop — Dry Run")
    print(f"{'=' * 50}")
    print(f"Name:           {config['name']}")
    print(f"Workspace:      {config['workspace']}")
    print(f"Spec directory: {config['spec_dir']}")
    print(f"Cycles:         {cycles}")
    print()
    print(f"Spec quality:   {config['spec_quality']}")
    print()
    print(f"Agent counts:")
    print(f"  Executives:   {agents['executives']}")
    print(f"  Managers:     {agents['managers']}")
    print(f"  Workers:      {agents['workers']}")
    print(f"  QA:           {agents['qa']}")
    print(f"  Docs:         {agents['docs']}")
    print(f"  Reviewers:    {agents['reviewers']}")
    print(f"  Antagonists:  {agents['antagonists']}")
    print(f"  Dreamers:     {agents['dreamers']}")
    print()
    print(f"Total stages:   {stage_info['total_stages']}")
    print(f"  Pre-loop:     {PRE_LOOP_STAGES}")
    print(f"  Loop:         {STAGES_PER_CYCLE} × {cycles} = {STAGES_PER_CYCLE * cycles}")
    print(f"  Post-loop:    {POST_LOOP_STAGES}")
    print(f"Total sheets:   {total_sheets} (after fan-out expansion)")
    print()

    # Per-cycle breakdown
    per_cycle = (
        agents["executives"]  # executive
        + agents["managers"]  # manager
        + agents["workers"]  # investigation
        + agents["qa"]  # test design
        + agents["workers"]  # implementation
        + agents["docs"]  # documentation
        + agents["reviewers"]  # review
        + 1  # synthesizer
        + agents["antagonists"]  # antagonist
        + agents["dreamers"]  # dreamers
    )
    pre_sheets = 1 + agents["workers"]  # intent + preprocessing
    post_sheets = 1 + (agents["workers"] + agents["qa"] + agents["docs"]) + 1 + 1 + 1

    print(f"Sheets per cycle: {per_cycle}")
    print(f"  Pre-loop sheets:  {pre_sheets}")
    print(f"  Loop sheets:      {per_cycle} × {cycles} = {per_cycle * cycles}")
    print(f"  Post-loop sheets: {post_sheets}")
    print(f"  Total:            {pre_sheets + per_cycle * cycles + post_sheets}")
    print()
    print(f"Skip conditions:  {len(stage_info['skip_when_command'])} (cycles 2-{cycles})")
    print(f"Dependencies:     {len(stage_info['dependencies'])} stage-level entries")


# ═══════════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════════


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mozart score YAML for an iterative development loop.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""\
Examples:
  %(prog)s config.yaml -o scores/my-feature.yaml
  %(prog)s config.yaml --dry-run
  %(prog)s config.yaml -o scores/quick.yaml --cycles 5
""",
    )
    parser.add_argument("config", help="Path to generator config YAML")
    parser.add_argument("-o", "--output", help="Output YAML file path")
    parser.add_argument("--cycles", type=int, help="Override cycle count")
    parser.add_argument("--dry-run", action="store_true", help="Print stats without generating")

    args = parser.parse_args()

    config = load_config(args.config, args.cycles)

    # Validate hierarchy
    hierarchy_errors = validate_hierarchy(config)
    if hierarchy_errors:
        print("Hierarchy validation errors:", file=sys.stderr)
        for err in hierarchy_errors:
            print(f"  - {err}", file=sys.stderr)
        raise SystemExit(1)

    stage_info = compute_stages(config)

    if args.dry_run:
        print_stats(config, stage_info)
        return

    if not args.output:
        parser.error("--output/-o is required unless --dry-run is specified")

    score = assemble_score(config)
    output = generate_yaml(score)

    # Write header comment + YAML
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    total_sheets = sum(
        stage_info["fan_out"].get(s, 1)
        for s in range(1, stage_info["total_stages"] + 1)
    )

    header = (
        f"# {config['name']}.yaml\n"
        f"#\n"
        f"# Iterative Development Loop Score\n"
        f"# Generated by scripts/generate-iterative-dev-loop.py\n"
        f"#\n"
        f"# Cycles: {config['cycles']}\n"
        f"# Total stages: {stage_info['total_stages']}\n"
        f"# Total sheets: {total_sheets} (after fan-out expansion)\n"
        f"#\n"
        f"# Pre-loop:  Intent alignment + Preprocessing\n"
        f"# Loop:      Executive → Manager → Investigation → Test Design (TDD) →\n"
        f"#            Implementation → Documentation → TDF Review →\n"
        f"#            Synthesizer → Antagonist → Memory Consolidation\n"
        f"# Post-loop: Doc Manager → Doc Analysis → Doc Plan →\n"
        f"#            Doc Execution → Doc Review\n"
        f"#\n\n"
    )

    with open(output_path, "w") as f:
        f.write(header)
        f.write(output)

    print(f"Generated: {output_path}")
    print(f"  Stages: {stage_info['total_stages']}, Sheets: {total_sheets}")
    print(f"  Cycles: {config['cycles']}, Skip conditions: {len(stage_info['skip_when_command'])}")


if __name__ == "__main__":
    main()
