package com.example.text

import java.util.Locale

data class Field(val name: String, val value: String)

fun render(fields: List<Field>, indent: Int = 0): String {
    val pad = " ".repeat(indent)
    return fields.joinToString("\n") { field ->
        "$pad${field.name} = ${quote(field.value)}"
    }
}

fun quote(text: String): String = "\"$text\""

fun describe(count: Int, total: Int): String {
    val percent = if (total == 0) 0.0 else count * 100.0 / total
    return "$count of $total (${"%.1f".format(Locale.ROOT, percent)}%)"
}

fun banner(title: String): String = """
    |== $title ==
    |${title.uppercase()}
""".trimMargin()

fun nested(items: List<Int>): String = "count=${items.size}, first=${items.firstOrNull() ?: -1}"

fun escaped(literal: String): String = "cost: \$$literal"

fun summary(rows: List<Field>): String = rows.joinToString("; ") { "${it.name}:${it.value}" }
