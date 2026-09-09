# Investigating the Impact of Compiler Bugs in zkVMs - Project artifacts

This repository holds the experimental artifacts for the above-stated MSc project.

| Directory | Contents |
| --- | --- |
| [`pipeline/`](./pipeline) | The automated differential-testing pipeline: 11 rustc issue specs, the instrumentation patches that make each fault announce itself, a 28-program corpus, the sweep scripts, the LLM agent roles, and 319 result cards under `findings/` (including those for the reproducers used as a sanity check). |
| [`synthetic-workloads/`](./synthetic-workloads) | 11 standalone Jolt programs, one per issue, each embedding a miscompiling code pattern in a realistic application (settlement, multisig, credential checks, ML inference) and recording the correct and miscompiled results. |

Refer to the README in each subdirectory for more details. 
