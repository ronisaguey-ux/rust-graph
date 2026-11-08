object Strings {
  val name = "world"
  val greeting = "hello, " + name
  val interpolated = s"hello, $name"
  val expression = s"sum: ${1 + 2}"
  val formatted = f"value: $name%s"
  val multiline = """line one
    |line two""".stripMargin
  val rawText = raw"no \n escapes"
  val quote = "a \"quoted\" word"
  val letter = 'x'
  val symbol = 'ready

  def show(): Unit = {
    println(greeting)
    println(interpolated)
    println(expression)
    println(formatted)
    println(multiline)
    println(rawText)
    println(quote)
    println(letter)
    println(symbol)
  }
}
