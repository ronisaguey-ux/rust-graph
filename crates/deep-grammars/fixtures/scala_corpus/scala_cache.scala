package com.example.cache

import java.time.Duration
import java.time.Instant

class Cache[K, V](ttl: Duration = Duration.ofMinutes(5)) {
  private val items = scala.collection.mutable.HashMap.empty[K, (V, Instant)]

  def get(key: K, load: K => V): V = items.get(key) match {
    case Some((value, expiresAt)) if expiresAt.isAfter(Instant.now()) => value
    case _ =>
      val value = load(key)
      items.put(key, (value, Instant.now().plus(ttl)))
      value
  }

  def put(key: K, value: V): Unit =
    items.put(key, (value, Instant.now().plus(ttl)))

  def size: Int = items.size

  def keys: List[K] = items.keys.toList.sortBy(key => key.toString)
}

object Cache {
  def of[K, V](): Cache[K, V] = new Cache[K, V]()

  def timed[K, V](minutes: Long): Cache[K, V] =
    new Cache[K, V](Duration.ofMinutes(minutes))
}
