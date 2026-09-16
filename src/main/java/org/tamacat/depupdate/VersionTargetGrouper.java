package org.tamacat.depupdate;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * business-rules.md BR-1a: groups raw {@code <dependency>} declarations into
 * {@link VersionTarget}s (step 1), and computes a {@link PropertyIndirectedTarget}'s
 * effective {@link VersionRangePolicy} (step 2).
 */
public final class VersionTargetGrouper {

	private static final Pattern PROPERTY_REFERENCE = Pattern.compile("^\\$\\{([^}]+)\\}$");

	private VersionTargetGrouper() {
	}

	/**
	 * BR-1a step 1: any two dependencies whose {@code <version>} text is the
	 * identical {@code ${propertyName}} reference form one
	 * {@link PropertyIndirectedTarget}, preserving first-encounter order for
	 * both the targets and each group's members. Every other dependency is
	 * its own {@link DirectDependencyTarget}.
	 *
	 * @param declarations the raw dependency declarations, in document order
	 * @param properties the {@code <properties>} map (property name -&gt; value)
	 * @throws IllegalStateException if a dependency references a property
	 *         that is not declared in {@code <properties>}
	 */
	public static List<VersionTarget> group(List<RawDependencyDeclaration> declarations, Map<String, String> properties) {
		Map<String, List<DependencyCoordinate>> membersByProperty = new LinkedHashMap<>();
		for (RawDependencyDeclaration d : declarations) {
			String propertyName = extractPropertyReference(d.versionText());
			if (propertyName != null) {
				membersByProperty.computeIfAbsent(propertyName, k -> new ArrayList<>()).add(d.coordinate());
			}
		}

		List<VersionTarget> result = new ArrayList<>();
		Set<String> emittedProperties = new HashSet<>();
		for (RawDependencyDeclaration d : declarations) {
			String propertyName = extractPropertyReference(d.versionText());
			if (propertyName == null) {
				result.add(new DirectDependencyTarget(d.coordinate(), d.versionText()));
				continue;
			}
			if (!emittedProperties.add(propertyName)) {
				continue; // already emitted as part of this property's group
			}
			String currentVersion = properties.get(propertyName);
			if (currentVersion == null) {
				throw new IllegalStateException(
						"Dependency " + d.coordinate() + " references property '" + propertyName
								+ "' which is not declared in <properties>");
			}
			result.add(new PropertyIndirectedTarget(propertyName, membersByProperty.get(propertyName), currentVersion));
		}
		return result;
	}

	/**
	 * BR-1a step 2: for a {@code DirectDependencyTarget}, its configured (or
	 * defaulted {@code PATCH_ONLY}) policy. For a {@code PropertyIndirectedTarget},
	 * the most restrictive policy among all members' individually-configured
	 * (or defaulted) policies.
	 */
	public static VersionRangePolicy effectivePolicy(VersionTarget target,
			Map<DependencyCoordinate, VersionRangePolicy> configuredPolicies) {
		if (target instanceof DirectDependencyTarget ddt) {
			return configuredPolicies.getOrDefault(ddt.coordinate(), VersionRangePolicy.PATCH_ONLY);
		}
		if (target instanceof PropertyIndirectedTarget pit) {
			List<VersionRangePolicy> memberPolicies = new ArrayList<>();
			for (DependencyCoordinate member : pit.members()) {
				memberPolicies.add(configuredPolicies.getOrDefault(member, VersionRangePolicy.PATCH_ONLY));
			}
			return VersionRangePolicy.mostRestrictive(memberPolicies);
		}
		throw new IllegalStateException("Unknown VersionTarget implementation: " + target.getClass());
	}

	private static String extractPropertyReference(String versionText) {
		Matcher m = PROPERTY_REFERENCE.matcher(versionText.trim());
		return m.matches() ? m.group(1) : null;
	}
}
