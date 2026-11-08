defmodule Scanner.Pipeline do
  require Logger

  def main(args) do
    case args do
      [] -> usage()
      [root | _] -> run(root)
    end
  end

  def usage do
    IO.puts("usage: pipeline <root>")
  end

  def run(root) do
    Logger.info("scanning #{root}")
    summary = Scanner.scan(root)
    report(summary)
  end

  def report(%{count: count, bytes: bytes}) do
    IO.puts("files: #{count}")
    IO.puts("bytes: #{bytes}")
  end

  defp unused(_value) do
    :ok
  end
end
