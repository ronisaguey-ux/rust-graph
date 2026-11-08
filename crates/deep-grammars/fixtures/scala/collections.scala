object Collections {
  val numbers: List[Int] = List(1, 2, 3, 4, 5)
  val empty: Map[String, Int] = Map.empty

  def headOrZero(values: List[Int]): Int = values match {
    case Nil => 0
    case first :: rest => first
  }

  def pair: (Int, String) = (1, "one")

  def lookup(values: Map[String, Int], key: String): Option[Int] = values.get(key)

  val doubled = numbers.map(x => x * 2)
  val evens = numbers.filter(x => x % 2 == 0)
  val total = numbers.foldLeft(0)((acc, n) => acc + n)
}
