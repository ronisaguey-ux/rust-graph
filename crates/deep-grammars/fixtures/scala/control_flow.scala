object Control {
  def classify(score: Int): String = {
    if (score >= 90) {
      "A"
    } else if (score >= 80) {
      "B"
    } else {
      "F"
    }
  }

  def describe(value: Int): String = value match {
    case 0 => "zero"
    case n if n > 0 => "positive"
    case _ => "negative"
  }

  var counter = 0
  while (counter < 3) {
    counter += 1
  }

  do {
    counter -= 1
  } while (counter > 0)

  val items = List(1, 2, 3)
  for (i <- items) {
    println(i)
  }

  for (i <- items; j <- items) {
    println(i + j)
  }
}
