trait Repository[A] {
  def find(id: Int): Option[A]
  def save(item: A): Unit
}

abstract class BaseRepository[A] extends Repository[A] {
  protected var items: List[A] = Nil

  def save(item: A): Unit = {
    items = item :: items
  }
}

class MemoryRepository extends BaseRepository[String] {
  override def find(id: Int): Option[String] = None
}

object Main {
  def main(args: Array[String]): Unit = {
    val repo = new MemoryRepository()
    repo.save("first")
    val found = repo.find(1)
    println(found)
  }
}
