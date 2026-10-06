import React from 'react';
import {Composition} from 'remotion';
import {Main} from './Main';
export const Root: React.FC = () => (
  <Composition id="Launch" component={Main} durationInFrames={4140} fps={60} width={1920} height={1080} />
);
