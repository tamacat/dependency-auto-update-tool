# dependency-auto-update-tool

A standalone, on-demand CLI tool for a single-module Maven project: it
detects candidate dependency updates, judges compatibility by actually
running `mvn clean verify` against each candidate, and applies
validated-safe updates to `pom.xml` with a configurable commit/push mode.

This tool is general-purpose -- it makes no assumption about which project
it runs against beyond "a git-managed, single-module Maven project." Which
branches it is allowed to operate on, and what JDK each one expects, is a
policy you declare in `config/tool.properties` (see
[Configuration](#configuration)); the tool ships no hardcoded branch list of
its own. It was originally built for, and ships pre-configured for, its
reference deployment: [`tamacat-httpd`](https://github.com/tamacat/tamacat-httpd)
(branches `v1.6` -> JDK 8, `v2.0-tc11` -> JDK 25).

## What it does, in order

1. **Pre-flight**: detects the currently checked-out branch, confirms it has
   a configured entry in `config/tool.properties` (see
   [Configuration](#configuration)), resolves that branch's configured JDK
   home (see [Setup](#setup) below), confirms it reports the expected major
   version, then confirms the working tree is clean. Any failure stops the
   run immediately, before any dependency is even looked at.
2. **Detection**: parses `pom.xml`'s direct, non-test-scope dependencies
   (excluding any dependency that shares the target project's own
   `groupId` -- a sibling artifact of the same project, not a third-party
   library), groups any that share a single `${property}` version reference
   (e.g. tamacat-httpd's own `tomcat.version`, shared by `tomcat-embed-core`
   and `tomcat-embed-jasper`) into one atomic update target, queries a Maven
   repository (Maven Central by default) for each target's available
   versions, and filters by the configured version-range policy
   (patch/minor/major, default patch-only).
3. **Validation**: for each candidate, temporarily rewrites `pom.xml`, runs
   `mvn clean verify` under the branch's configured JDK, and classifies the
   result as safe (`GREEN`), genuinely incompatible (`INCOMPATIBLE`), or an
   infrastructure hiccup unrelated to the candidate (`INFRA_FAILURE`).
4. **Applying**: a `GREEN` candidate is committed. Depending on the
   configured mode, the tool either stops there (the default) or also
   pushes.
5. **Reporting**: everything above is written to the console and to a
   structured report file.

See [`tamacat-httpd`](https://github.com/tamacat/tamacat-httpd)'s
`aidlc/spaces/default/intents/260903-dependency-auto-update/` AI-DLC record
for this tool's full original design rationale (requirements, architecture
decisions, business rules) -- this tool was designed and generated inside
that project before being extracted into its own repository.

## Building

Building and running this tool requires JDK 25 or later
(`maven.compiler.release` is 25). This is the JDK this tool itself runs on,
independent of the JDKs it validates each target branch with (see
[Setup](#setup-once-per-machine-before-first-use)).

```
mvn clean package
```

Produces `target/dependency-auto-update-tool-1.0.0.jar` (a plain jar with a
`Main-Class` manifest entry -- this tool has no runtime dependencies beyond
the JDK, so no shading/fat-jar step is needed).

## Setup (once per machine, before first use)

The tool validates dependency updates by actually building the target
project under **that branch's own JDK**, which is deliberately independent
of whatever JDK this tool's own code happens to run on. You must tell it
where each configured branch's JDK lives on your machine:

```
cp config/jdk-home.local.properties.template config/jdk-home.local.properties
```

Then edit `config/jdk-home.local.properties` and set real paths for each
branch declared in `config/tool.properties`, e.g. (tamacat-httpd's own
reference values):

```
jdkHome.v1.6=/opt/amazon-corretto-8
jdkHome.v2.0-tc11=/opt/amazon-corretto-25
```

**This file is deliberately not committed to git** (see `.gitignore`) -- a
JDK installation path is a fact about one machine, not a repo-wide policy
every clone should share. If you skip this step, point it at the wrong JDK,
or check out a branch with no entry in `config/tool.properties` at all, the
tool fails fast at pre-flight with a clear `BRANCH_JDK_MISMATCH` error
rather than silently validating under the wrong Java version.

## Configuration

Two independent, `.properties`-format config files (chosen at Code
Generation -- no new dependency for YAML parsing):

| File | Committed? | Holds |
|---|---|---|
| `config/tool.properties` | Yes | Commit/push mode; which branches this tool may operate on and each one's expected JDK major version; per-dependency version-range policy; repository URL/timeout/report-dir overrides |
| `config/jdk-home.local.properties` | **No** (gitignored) | Per-branch JDK home paths -- machine-local, see Setup above |

`config/tool.properties` ships pre-configured for this tool's reference
deployment (tamacat-httpd's two branches, safe commit-mode/policy defaults)
-- see the file's own comments for every key and its exact syntax. **Adopting
this tool for a different project**: edit the `branch.<name>.expectedJdkMajor`
entries to declare that project's own branches and their JDK targets; a
branch with no entry is refused at pre-flight rather than silently
validated. Note that a `policy.<groupId>:<artifactId>` key's colon **must**
be escaped as `\:` in the file -- `java.util.Properties` treats an unescaped
`:` as a key/value separator, same as `=` (see the file's own comment for
why, and `ToolConfigurationTest` for the regression test).

## Running

```
java -jar target/dependency-auto-update-tool-1.0.0.jar <repoRoot>
```

The one required argument is the path to the target project's checkout to
operate on (its `pom.xml` must sit directly at `<repoRoot>/pom.xml`, i.e. a
single-module project or a specific module directory -- this tool does not
walk a multi-module reactor). Two optional arguments override the config
file paths (both otherwise resolved relative to the current working
directory):

```
java -jar target/dependency-auto-update-tool-1.0.0.jar <repoRoot> [toolConfigPath] [jdkHomeConfigPath]
```

The report file lands under `reports/` (relative to the current working
directory) -- gitignored, one file per run.

## Commit/push modes

- **`LOCAL_COMMIT_MANUAL_APPROVAL`** (default): a `GREEN` candidate is
  committed locally; the maintainer reviews and pushes manually. This is the
  primary mitigation against this tool's central, disclosed risk: a
  compromised-but-functionally-compatible dependency release would still
  pass `mvn clean verify` and be classified `GREEN` -- the manual-review
  step before `origin` is what catches what the build cannot.
- **`FULL_LOOP_AUTO_PUSH`** (opt-in): also pushes automatically once the
  commit succeeds. Materially increases supply-chain exposure by removing
  that review step -- opt in deliberately.

Whichever mode is configured, a **failed commit stops the run** (no further
candidates are attempted this invocation) so that a later candidate's
successful commit can never silently bundle an earlier, still-uncommitted
change. If you see a run fail pre-flight with `DIRTY_WORKING_TREE`, check
`git status`/`git diff` first -- it is very likely an earlier run's
unresolved commit failure, not a new problem.

## Exit code contract

- `0`: every target either had nothing new to apply, or was successfully
  applied.
- Non-zero (`1`): the run stopped at pre-flight, or at least one target
  needs attention -- a repository query failed, a candidate was
  incompatible or hit an infra failure during validation, a commit failed,
  or a commit succeeded but the subsequent push failed.
- `2`: usage error (missing required argument, unreadable/invalid config file).

## What this tool deliberately does not do

- No scheduling/cron trigger -- this is on-demand logic only; run it however
  and whenever you like (cron, a manual invocation, a CI job).
- No transitive-dependency scanning -- only the target project's own direct,
  non-test-scope `pom.xml` dependencies are considered.
- No vulnerability-database integration -- this tool's compatibility
  judgment is "does the real build/test suite still pass," not "is this
  version free of known CVEs."
- No multi-module reactor support -- it reads exactly one `pom.xml` at the
  given `repoRoot`.
