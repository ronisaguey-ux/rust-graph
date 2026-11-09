package com.example.text

object Render {
  case class Field(name: String, value: String)

  def quote(text: String): String = "\"" + text + "\""

  def render(fields: List[Field], indent: Int = 0): String = {
    val pad = " " * indent
    fields.map(field => s"$pad${field.name} = ${quote(field.value)}").mkString("\n")
  }

  def describe(count: Int, total: Int): String = {
    val percent = if (total == 0) 0.0 else count * 100.0 / total
    f"$count of $total (${percent}%.1f%%)"
  }

  def nested(items: List[Int]): String =
    s"count=${items.size}, first=${items.headOption.getOrElse(-1)}"

  def banner(title: String): String = s"""
    |== $title ==
    |${title.toUpperCase}
""".stripMargin

  def escaped(literal: String): String = s"cost: $$$literal"
}
