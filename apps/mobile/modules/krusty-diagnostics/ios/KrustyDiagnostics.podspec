Pod::Spec.new do |s|
  s.name           = 'KrustyDiagnostics'
  s.version        = '1.0.0'
  s.summary        = 'Content-free MetricKit diagnostics for Mitsuro internal builds'
  s.description    = 'Collects bounded, content-free MetricKit summaries for authenticated Honey diagnostics.'
  s.author         = 'Honeycomb Technologies'
  s.homepage       = 'https://github.com/honeycomb-Technologies/Krusty'
  s.platform       = :ios, '15.1'
  s.source         = { git: 'https://github.com/honeycomb-Technologies/Krusty.git' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # Swift/Objective-C compatibility
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.source_files = "**/*.{h,m,mm,swift,hpp,cpp}"
end
