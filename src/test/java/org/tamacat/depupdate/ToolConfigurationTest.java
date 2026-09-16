package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.OptionalInt;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * BR-3 (generalized): a branch's expected JDK major version is a repo-wide
 * policy fact declared in {@code tool.properties} (a {@code branch.<name>.
 * expectedJdkMajor=<n>} entry), not a value this tool hardcodes for any one
 * adopting project.
 */
class ToolConfigurationTest {

	@Test
	void defaults_hasNoBranchPolicy_andSafeFallbacks() {
		ToolConfiguration config = ToolConfiguration.defaults();

		assertEquals(CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL, config.commitPushMode());
		assertEquals(OptionalInt.empty(), config.expectedJdkMajorVersion("master"),
				"a tool that ships no hardcoded branch list recognizes no branch until configured");
	}

	@Test
	void missingConfigFile_fallsBackToDefaults(@TempDir Path dir) throws IOException {
		ToolConfiguration config = ToolConfiguration.loadFromProperties(dir.resolve("does-not-exist.properties"));

		assertEquals(ToolConfiguration.defaults(), config);
	}

	@Test
	void loadFromProperties_parsesBranchPolicy_includingADottedBranchName(@TempDir Path dir) throws IOException {
		// "v2.0-tc11" itself contains a dot -- the parser must trim the known
		// branch. / .expectedJdkMajor prefix/suffix rather than splitting on '.',
		// or it would misparse this branch name.
		Path propsFile = dir.resolve("tool.properties");
		Files.writeString(propsFile, """
				branch.master.expectedJdkMajor=8
				branch.v2.0-tc11.expectedJdkMajor=25
				""");

		ToolConfiguration config = ToolConfiguration.loadFromProperties(propsFile);

		assertEquals(OptionalInt.of(8), config.expectedJdkMajorVersion("master"));
		assertEquals(OptionalInt.of(25), config.expectedJdkMajorVersion("v2.0-tc11"));
		assertEquals(OptionalInt.empty(), config.expectedJdkMajorVersion("some-other-branch"));
	}

	@Test
	void loadFromProperties_invalidExpectedJdkMajor_throwsWithHelpfulMessage(@TempDir Path dir) throws IOException {
		Path propsFile = dir.resolve("tool.properties");
		Files.writeString(propsFile, "branch.master.expectedJdkMajor=not-a-number\n");

		IOException ex = assertThrows(IOException.class, () -> ToolConfiguration.loadFromProperties(propsFile));

		assertTrue(ex.getMessage().contains("master"), "message should name the offending branch");
		assertTrue(ex.getMessage().contains("branch.<branchName>.expectedJdkMajor"),
				"message should show the expected key shape");
	}

	@Test
	void loadFromProperties_parsesEveryFieldTogether(@TempDir Path dir) throws IOException {
		// The colon in "policy.<groupId>:<artifactId>" MUST be escaped ("\:") in
		// the properties file -- java.util.Properties treats an unescaped ':' as
		// a key/value separator (same as '='), so an unescaped policy key like
		// "policy.org.example:widget=MINOR" parses as key="policy.org.example",
		// value="widget=MINOR", never reaching this class's own ':'-splitting
		// logic at all. Verified directly against java.util.Properties.load().
		Path propsFile = dir.resolve("tool.properties");
		Files.writeString(propsFile, """
				commitPushMode=FULL_LOOP_AUTO_PUSH
				repositoryBaseUrl=https://example.invalid/repo/
				repositoryQueryTimeoutSeconds=30
				reportDir=custom-reports
				branch.main.expectedJdkMajor=21
				policy.org.example\\:widget=MINOR
				""");

		ToolConfiguration config = ToolConfiguration.loadFromProperties(propsFile);

		assertEquals(CommitPushMode.FULL_LOOP_AUTO_PUSH, config.commitPushMode());
		assertEquals("https://example.invalid/repo/", config.repositoryBaseUrl());
		assertEquals(Duration.ofSeconds(30), config.repositoryQueryTimeout());
		assertEquals(Path.of("custom-reports"), config.reportDirectory());
		assertEquals(OptionalInt.of(21), config.expectedJdkMajorVersion("main"));
		assertEquals(VersionRangePolicy.MINOR,
				config.versionRangePolicies().get(new DependencyCoordinate("org.example", "widget")));
	}

	@Test
	void loadFromProperties_unescapedColonInPolicyKey_failsWithAnActionableMessage(@TempDir Path dir)
			throws IOException {
		// Documents the java.util.Properties pitfall above as an explicit,
		// regression-tested fact: java.util.Properties itself absorbs the
		// unescaped ':' as the key/value separator before this class ever sees
		// a "policy." key containing one, leaving a colon-less remainder
		// ("org.example") that loadFromProperties correctly rejects (fail loud,
		// not silently ignored) -- verify the message actually names the
		// escaping fix, not just "invalid key".
		Path propsFile = dir.resolve("tool.properties");
		Files.writeString(propsFile, "policy.org.example:widget=MINOR\n");

		IOException ex = assertThrows(IOException.class, () -> ToolConfiguration.loadFromProperties(propsFile));

		assertTrue(ex.getMessage().contains("\\:"), "message should point at the required escaping fix");
	}

	@Test
	void withExpectedJdkMajor_returnsIndependentCopy_originalUnchanged() {
		ToolConfiguration original = ToolConfiguration.defaults();
		ToolConfiguration updated = original.withExpectedJdkMajor("develop", 17);

		assertEquals(OptionalInt.empty(), original.expectedJdkMajorVersion("develop"));
		assertEquals(OptionalInt.of(17), updated.expectedJdkMajorVersion("develop"));
	}
}
