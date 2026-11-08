object Patterns {
  def sign(value: Int): String = value match {
    case 0 => "zero"
    case n if n < 0 => "negative"
    case _ => "positive"
  }

  def classify(pair: (Int, String)): String = pair match {
    case (0, label) => label
    case (_, label) => label
  }

  def safeDivide(a: Int, b: Int): Int = {
    try {
      a / b
    } catch {
      case e: ArithmeticException => 0
    } finally {
      println("done")
    }
  }

  def run(): Unit = {
    val result = safeDivide(10, 2)
    println(result)
  }
}
