defmodule Broken do
  def scan(root) do
    case File.ls(root) do
      {:ok, entries} -> entries
    end
  end
