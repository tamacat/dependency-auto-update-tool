package org.tamacat.depupdate;

import java.nio.file.Path;
import java.util.Optional;

/**
 * Resolves the machine-local, gitignored JDK-home path configured for a
 * given branch (NFR Requirements Q1a; tech-stack-decisions.md). Distinct
 * from {@link ToolConfiguration}, which is repo-checked-in.
 */
public interface JdkHomeResolver {

	Optional<Path> resolve(String branch);
}
