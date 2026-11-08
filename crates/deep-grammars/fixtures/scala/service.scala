package com.example.service

import scala.collection.mutable.ListBuffer
import java.util.UUID

case class User(id: String, name: String, active: Boolean)

class UserService {
  private val users = new ListBuffer[User]()

  def add(name: String): User = {
    val user = User(UUID.randomUUID().toString, name, true)
    users += user
    user
  }

  def find(id: String): Option[User] = {
    users.find(user => user.id == id)
  }

  def activeUsers(): List[User] = {
    users.filter(user => user.active).toList
  }
}

object UserService {
  def apply(): UserService = new UserService()
}
