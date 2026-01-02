# frozen_string_literal: true

require_relative "rrprof/version"

module Rrprof
  def self.visualizer_path
    exe = "rrprof#{RbConfig::CONFIG["EXEEXT"]}"
    File.expand_path("../libexec/#{exe}", __dir__)
  end
end

require "rrprof/rrprof"
