package org.tamacat.depupdate;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A minimal JSON reader, so reading the vulnerability database's responses
 * needs no new dependency (this tool has none beyond the JDK).
 *
 * <p>Objects become {@link Map}, arrays {@link List}, strings {@link String},
 * numbers {@link BigDecimal}, and {@code true}/{@code false}/{@code null}
 * {@link Boolean} / {@code null}. Input is fully validated; anything
 * malformed is an {@link IllegalArgumentException}.
 */
final class MiniJson {

	/** Guards against stack exhaustion on hostile, deeply nested input. */
	private static final int MAX_DEPTH = 64;

	private final String text;
	private int pos;

	private MiniJson(String text) {
		this.text = text;
	}

	static Object parse(String text) {
		MiniJson p = new MiniJson(text);
		Object value = p.value(0);
		p.skipWhitespace();
		if (p.pos != text.length()) {
			throw p.error("unexpected trailing content");
		}
		return value;
	}

	private Object value(int depth) {
		if (depth > MAX_DEPTH) {
			throw error("nested too deeply");
		}
		skipWhitespace();
		if (pos >= text.length()) {
			throw error("unexpected end of input");
		}
		char c = text.charAt(pos);
		return switch (c) {
			case '{' -> object(depth);
			case '[' -> array(depth);
			case '"' -> string();
			case 't' -> literal("true", Boolean.TRUE);
			case 'f' -> literal("false", Boolean.FALSE);
			case 'n' -> literal("null", null);
			default -> number();
		};
	}

	private Map<String, Object> object(int depth) {
		Map<String, Object> map = new LinkedHashMap<>();
		pos++; // '{'
		skipWhitespace();
		if (peek('}')) {
			pos++;
			return map;
		}
		while (true) {
			skipWhitespace();
			if (!peek('"')) {
				throw error("expected a string key");
			}
			String key = string();
			skipWhitespace();
			expect(':');
			map.put(key, value(depth + 1));
			skipWhitespace();
			if (peek(',')) {
				pos++;
			} else {
				expect('}');
				return map;
			}
		}
	}

	private List<Object> array(int depth) {
		List<Object> list = new ArrayList<>();
		pos++; // '['
		skipWhitespace();
		if (peek(']')) {
			pos++;
			return list;
		}
		while (true) {
			list.add(value(depth + 1));
			skipWhitespace();
			if (peek(',')) {
				pos++;
			} else {
				expect(']');
				return list;
			}
		}
	}

	private String string() {
		pos++; // opening quote
		StringBuilder sb = new StringBuilder();
		while (pos < text.length()) {
			char c = text.charAt(pos++);
			if (c == '"') {
				return sb.toString();
			}
			if (c != '\\') {
				sb.append(c);
				continue;
			}
			if (pos >= text.length()) {
				break;
			}
			char e = text.charAt(pos++);
			switch (e) {
				case '"', '\\', '/' -> sb.append(e);
				case 'b' -> sb.append('\b');
				case 'f' -> sb.append('\f');
				case 'n' -> sb.append('\n');
				case 'r' -> sb.append('\r');
				case 't' -> sb.append('\t');
				case 'u' -> {
					if (pos + 4 > text.length()) {
						throw error("truncated \\u escape");
					}
					try {
						sb.append((char) Integer.parseInt(text.substring(pos, pos + 4), 16));
					} catch (NumberFormatException ex) {
						throw error("invalid \\u escape");
					}
					pos += 4;
				}
				default -> throw error("invalid escape \\" + e);
			}
		}
		throw error("unterminated string");
	}

	private BigDecimal number() {
		int start = pos;
		while (pos < text.length() && "+-0123456789.eE".indexOf(text.charAt(pos)) >= 0) {
			pos++;
		}
		if (start == pos) {
			throw error("unexpected character '" + text.charAt(pos) + "'");
		}
		try {
			return new BigDecimal(text.substring(start, pos));
		} catch (NumberFormatException e) {
			throw error("invalid number");
		}
	}

	private Object literal(String word, Object value) {
		if (!text.startsWith(word, pos)) {
			throw error("invalid literal");
		}
		pos += word.length();
		return value;
	}

	private void skipWhitespace() {
		while (pos < text.length() && Character.isWhitespace(text.charAt(pos))) {
			pos++;
		}
	}

	private boolean peek(char c) {
		return pos < text.length() && text.charAt(pos) == c;
	}

	private void expect(char c) {
		if (!peek(c)) {
			throw error("expected '" + c + "'");
		}
		pos++;
	}

	private IllegalArgumentException error(String message) {
		return new IllegalArgumentException("Invalid JSON at offset " + pos + ": " + message);
	}
}
