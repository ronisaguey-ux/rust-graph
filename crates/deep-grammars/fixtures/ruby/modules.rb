# Modules: requires, mixins and namespaced calls.
require "json"
require_relative "../lib/helper"
load "config.rb"

module Utilities
  def self.parse(text)
    JSON.parse(text)
  end
end

module Formatting
  def format_value(value)
    value.to_s
  end
end

class Report
  include Formatting
  extend Utilities
  prepend Comparable

  def render(records)
    records.map { |record| format_value(record) }.join(", ")
  end
end
