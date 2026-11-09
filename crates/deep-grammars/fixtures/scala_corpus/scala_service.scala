package com.example.jobs

import java.time.Instant
import scala.util.{Failure, Success, Try}

case class Job(id: String, payload: String, submittedAt: Instant = Instant.now())

trait JobStore {
  def put(job: Job): Unit
  def get(id: String): Option[Job]
  def all: List[Job]
}

class InMemoryStore extends JobStore {
  private var jobs = Map.empty[String, Job]

  override def put(job: Job): Unit = {
    jobs = jobs + (job.id -> job)
  }

  override def get(id: String): Option[Job] = jobs.get(id)

  override def all: List[Job] = jobs.values.toList
}

object Runner {
  def run(job: Job, handler: Job => Try[String]): String =
    handler(job) match {
      case Success(result) => s"ok: $result"
      case Failure(error) => s"failed: ${error.getMessage}"
    }

  def firstId(jobs: List[Job]): String =
    jobs.headOption.map(_.id).getOrElse("")

  def index(jobs: List[Job]): Map[String, Job] =
    jobs.map(job => job.id -> job).toMap
}
