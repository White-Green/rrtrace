# frozen_string_literal: true

require_relative "rrtrace/version"

module Rrtrace
  def self.visualizer_path
    exe = "rrtrace#{RbConfig::CONFIG["EXEEXT"]}"
    File.expand_path("../libexec/#{exe}", __dir__)
  end
end

require "rrtrace/rrtrace"
