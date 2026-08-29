Pod::Spec.new do |s|
  s.name           = 'MitsuroLiquidGlass'
  s.version        = '1.0.0'
  s.summary        = 'Native Liquid Glass artwork for Mitsuro controls'
  s.description    = 'A single non-interactive SwiftUI glass container driven by React Native geometry and progress.'
  s.author         = 'Honeycomb Technologies'
  s.homepage       = 'https://github.com/honeycomb-Technologies/Mitsuro'
  s.platform       = :ios, '15.1'
  s.source         = { git: 'https://github.com/honeycomb-Technologies/Mitsuro.git' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.source_files = '**/*.{h,m,mm,swift,hpp,cpp}'
end
