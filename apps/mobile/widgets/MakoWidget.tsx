import { createWidget } from 'expo-widgets';
import {
  Text,
  VStack,
  HStack,
  Spacer,
  Image,
  ProgressView,
  AccessoryWidgetBackground,
} from '@expo/ui/swift-ui';
import {
  frame,
  font,
  foregroundStyle,
  padding,
  cornerRadius,
  background,
  opacity,
  lineLimit,
} from '@expo/ui/swift-ui/modifiers';
import type { WidgetEnvironment } from 'expo-widgets';

interface MakoProps {
  status: 'active' | 'idle' | 'running' | 'offline';
  lastUpdate: string;
  briefing: string;
  taskName?: string;
  taskProgress?: number;
  completedTasks: number;
  totalTasks: number;
  serverConnected: boolean;
}

function MakoWidget(props: MakoProps, env: WidgetEnvironment) {
  'widget';

  const { status, lastUpdate, briefing, taskName, taskProgress, completedTasks, totalTasks, serverConnected } = props;
  const family = env.widgetFamily;

  const statusColor = status === 'active' || status === 'running'
    ? '#4ade80'
    : status === 'idle'
    ? '#fbbf24'
    : '#ef4444';

  const statusLabel = status === 'running' ? 'Running' : status === 'active' ? 'Active' : status === 'idle' ? 'Idle' : 'Offline';

  // Lock screen — circular
  if (family === 'accessoryCircular') {
    return (
      <AccessoryWidgetBackground>
        <VStack alignment="center">
          <Image systemName={status === 'offline' ? 'bolt.slash.fill' : 'bolt.fill'} modifiers={[font({ size: 16 })]} />
          <Text modifiers={[font({ size: 8, weight: 'semibold' })]}>Mako</Text>
        </VStack>
      </AccessoryWidgetBackground>
    );
  }

  // Lock screen — rectangular
  if (family === 'accessoryRectangular') {
    return (
      <VStack alignment="leading">
        <HStack>
          <Image systemName="bolt.fill" modifiers={[font({ size: 10 })]} />
          <Text modifiers={[font({ size: 12, weight: 'semibold' })]}>Mako — {statusLabel}</Text>
        </HStack>
        <Text modifiers={[font({ size: 11 }), lineLimit(2), opacity(0.7)]}>
          {briefing || 'No updates yet'}
        </Text>
      </VStack>
    );
  }

  // Lock screen — inline
  if (family === 'accessoryInline') {
    return (
      <HStack>
        <Image systemName="bolt.fill" />
        <Text>Mako: {statusLabel}</Text>
      </HStack>
    );
  }

  // Home screen — small (2x2)
  if (family === 'systemSmall') {
    return (
      <VStack alignment="leading" modifiers={[
        padding({ all: 14 }),
        background('#0b1119'),
        foregroundStyle('#ffffff'),
      ]}>
        <HStack>
          <Image systemName="bolt.fill" modifiers={[foregroundStyle(statusColor), font({ size: 14 })]} />
          <Text modifiers={[font({ size: 13, weight: 'bold' })]}>Mako</Text>
          <Spacer />
          <Text modifiers={[font({ size: 10 }), foregroundStyle(statusColor)]}>{statusLabel}</Text>
        </HStack>
        <Spacer />
        <Text modifiers={[font({ size: 12 }), lineLimit(3), opacity(0.8)]}>
          {briefing || 'Waiting for updates...'}
        </Text>
        <Spacer />
        <Text modifiers={[font({ size: 9 }), opacity(0.5)]}>{lastUpdate}</Text>
      </VStack>
    );
  }

  // Home screen — medium (4x2)
  if (family === 'systemMedium') {
    return (
      <VStack alignment="leading" modifiers={[
        padding({ all: 14 }),
        background('#0b1119'),
        foregroundStyle('#ffffff'),
      ]}>
        <HStack>
          <Image systemName="bolt.fill" modifiers={[foregroundStyle(statusColor), font({ size: 16 })]} />
          <Text modifiers={[font({ size: 15, weight: 'bold' })]}>Mako Agent</Text>
          <Spacer />
          <Text modifiers={[
            font({ size: 11, weight: 'medium' }),
            foregroundStyle('#0b1119'),
            padding({ horizontal: 8, vertical: 3 }),
            background(statusColor),
            cornerRadius(6),
          ]}>{statusLabel}</Text>
        </HStack>

        <Spacer />

        <Text modifiers={[font({ size: 13 }), lineLimit(2), opacity(0.85)]}>
          {briefing || 'No recent briefing'}
        </Text>

        {taskName && taskProgress !== undefined && (
          <VStack alignment="leading" spacing={4} modifiers={[padding({ top: 6 })]}>
            <Text modifiers={[font({ size: 11 }), opacity(0.6)]}>{taskName}</Text>
            <ProgressView value={taskProgress} modifiers={[foregroundStyle('#ff6b35')]} />
          </VStack>
        )}

        <Spacer />

        <HStack>
          <Text modifiers={[font({ size: 10 }), opacity(0.5)]}>{lastUpdate}</Text>
          <Spacer />
          <Text modifiers={[font({ size: 10 }), opacity(0.5)]}>
            {completedTasks}/{totalTasks} tasks
          </Text>
        </HStack>
      </VStack>
    );
  }

  // Home screen — large (4x4)
  return (
    <VStack alignment="leading" modifiers={[
      padding({ all: 16 }),
      background('#0b1119'),
      foregroundStyle('#ffffff'),
    ]}>
      <HStack>
        <Image systemName="bolt.fill" modifiers={[foregroundStyle(statusColor), font({ size: 18 })]} />
        <Text modifiers={[font({ size: 17, weight: 'bold' })]}>Mako Agent</Text>
        <Spacer />
        <VStack alignment="trailing">
          <Text modifiers={[
            font({ size: 11, weight: 'medium' }),
            foregroundStyle('#0b1119'),
            padding({ horizontal: 8, vertical: 3 }),
            background(statusColor),
            cornerRadius(6),
          ]}>{statusLabel}</Text>
          {serverConnected && (
            <HStack spacing={4} modifiers={[padding({ top: 4 })]}>
              <Image systemName="circle.fill" modifiers={[foregroundStyle('#4ade80'), font({ size: 6 })]} />
              <Text modifiers={[font({ size: 9 }), opacity(0.5)]}>Connected</Text>
            </HStack>
          )}
        </VStack>
      </HStack>

      <Spacer />

      <VStack alignment="leading" spacing={6}>
        <Text modifiers={[font({ size: 12, weight: 'semibold' }), opacity(0.5)]}>DAILY BRIEFING</Text>
        <Text modifiers={[font({ size: 14 }), lineLimit(4), opacity(0.9)]}>
          {briefing || 'No briefing available. Mako will generate one when active.'}
        </Text>
      </VStack>

      {taskName && taskProgress !== undefined && (
        <VStack alignment="leading" spacing={4} modifiers={[padding({ top: 12 })]}>
          <HStack>
            <Text modifiers={[font({ size: 11, weight: 'medium' })]}>Current Task</Text>
            <Spacer />
            <Text modifiers={[font({ size: 11 }), opacity(0.5)]}>{Math.round(taskProgress * 100)}%</Text>
          </HStack>
          <Text modifiers={[font({ size: 12 }), opacity(0.7)]}>{taskName}</Text>
          <ProgressView value={taskProgress} modifiers={[foregroundStyle('#ff6b35')]} />
        </VStack>
      )}

      <Spacer />

      <HStack>
        <Text modifiers={[font({ size: 10 }), opacity(0.4)]}>{lastUpdate}</Text>
        <Spacer />
        <Text modifiers={[font({ size: 10 }), opacity(0.4)]}>
          {completedTasks}/{totalTasks} tasks completed
        </Text>
      </HStack>
    </VStack>
  );
}

export default createWidget('MakoWidget', MakoWidget);
