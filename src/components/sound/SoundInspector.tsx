import React, { memo, useState, useCallback, useEffect } from 'react';
import {
  Check,
  Tag,
  X,
  Play,
  RefreshCw,
  Trash2,
  AlertTriangle,
  BookmarkCheck,
} from 'lucide-react';
import { Waveform } from '../waveform/Waveform';
import {
  call,
  duration,
  listClips,
  rebindClip,
  deleteClip,
  frameToSeconds,
  playClip,
} from '../../api';
import type { Sound, PlaybackStatus, Clip } from '../../types';

interface SoundInspectorProps {
  sound: Sound;
  playback: PlaybackStatus | null;
  onPlay: (sound: Sound) => Promise<unknown>;
  onSeek: (pos: number) => Promise<unknown>;
  onClose: () => void;
  onSave: (tags: string[], comment: string) => Promise<void>;
  onError: (e: string) => void;
}

/** Extract pitch/key info from profile tags if available. */
function extractPitchInfo(sound: Sound): string | null {
  const tags = sound.profile?.tags || [];
  for (const t of tags) {
    // Common key tag formats: key_Cm, key_A, musical_key_Dm
    const match = t.match(/^(?:key_|musical_key_)([A-Ga-g][#b♯♭]?m?)$/i);
    if (match) return match[1];
  }
  return null;
}

export const SoundInspector = memo(function SoundInspector({
  sound,
  playback,
  onPlay,
  onSeek,
  onClose,
  onSave,
  onError,
}: SoundInspectorProps) {
  const [tags, setTags] = useState(sound.user_tags);
  const [tag, setTag] = useState('');
  const [comment, setComment] = useState(sound.comment);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [selection, setSelection] = useState<{
    start: number;
    end: number;
  } | null>(null);
  const [clips, setClips] = useState<Clip[]>([]);
  const [loadingClips, setLoadingClips] = useState(false);
  const [rebindingId, setRebindingId] = useState<string | null>(null);

  const p = sound.profile;
  const isThisSound = playback?.sound_id === sound.id;
  const currentPos = isThisSound ? playback?.position_seconds || 0 : 0;
  const pitchKey = extractPitchInfo(sound);

  useEffect(() => {
    let active = true;
    setLoadingClips(true);
    listClips(sound.id)
      .then((res) => {
        if (active) setClips(Array.isArray(res) ? res : []);
      })
      .catch(() => {
        if (active) setClips([]);
      })
      .finally(() => {
        if (active) setLoadingClips(false);
      });
    return () => {
      active = false;
    };
  }, [sound.id]);

  const safeClips = Array.isArray(clips) ? clips : [];

  const handleClipSaved = useCallback((newClip: Clip) => {
    setClips((prev) => {
      const idx = prev.findIndex((c) => c.id === newClip.id);
      if (idx >= 0) {
        const copy = [...prev];
        copy[idx] = newClip;
        return copy;
      }
      return [...prev, newClip];
    });
  }, []);

  const handleLoadClip = useCallback(
    (clip: Clip) => {
      const rate = clip.recipe.source_sample_rate_hz || p?.sample_rate || 48000;
      const start = frameToSeconds(clip.recipe.start_frame, rate);
      const end = frameToSeconds(clip.recipe.end_frame, rate);
      setSelection({ start, end });
      handleSeek(start);
    },
    [p?.sample_rate],
  );

  const handleRebind = useCallback(
    async (clipId: string) => {
      setRebindingId(clipId);
      try {
        const updated = await rebindClip(clipId);
        setClips((prev) => prev.map((c) => (c.id === clipId ? updated : c)));
      } catch (err) {
        onError(String(err));
      } finally {
        setRebindingId(null);
      }
    },
    [onError],
  );

  const handleDeleteClip = useCallback(
    async (clipId: string) => {
      try {
        await deleteClip(clipId);
        setClips((prev) => prev.filter((c) => c.id !== clipId));
      } catch (err) {
        onError(String(err));
      }
    },
    [onError],
  );

  const addTag = useCallback(() => {
    const next = tag.replaceAll('_', ' ').trim();
    if (next && !tags.some((t) => t.toLowerCase() === next.toLowerCase())) {
      setTags((prev) => [...prev, next]);
    }
    setTag('');
    setSaved(false);
  }, [tag, tags]);

  const removeTag = useCallback(
    (t: string) => {
      setTags((prev) => prev.filter((x) => x !== t));
      setSaved(false);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      await onSave(tags, comment);
      setSaved(true);
    } catch (e) {
      onError(String(e));
    } finally {
      setSaving(false);
    }
  }, [tags, comment, onSave, onError]);

  const handleSeek = useCallback(
    (sec: number) => {
      if (isThisSound) {
        void onSeek(sec);
      } else {
        void onPlay(sound).then(() => onSeek(sec));
      }
    },
    [isThisSound, onSeek, onPlay, sound],
  );

  return (
    <aside className="inspector" aria-labelledby="inspector-title">
      <div className="inspector-heading">
        <span>DETAILS</span>
        <button
          className="icon-button"
          title="Close details"
          aria-label="Close details"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </div>

      <h2 id="inspector-title">{sound.title}</h2>
      <small className="muted path-text">{sound.relative_path}</small>

      {p && (
        <>
          <Waveform
            soundId={sound.id}
            soundHash={sound.content_hash}
            peaks={p.waveform}
            duration={p.duration}
            sampleRate={p.sample_rate}
            channels={p.channels}
            playbackPosition={currentPos}
            selection={selection}
            onSelectionChange={setSelection}
            onClipSaved={handleClipSaved}
            onSeek={handleSeek}
          />

          <div className="audio-facts">
            <span>{duration(p.duration)}</span>
            <span>{(p.sample_rate / 1000).toFixed(1)} kHz</span>
            <span>
              {p.channel_layout ||
                (p.channels === 1
                  ? 'mono'
                  : p.channels === 2
                    ? 'stereo'
                    : `${p.channels} ch`)}
            </span>
            {pitchKey && (
              <span className="pitch-badge" title="Detected musical key">
                🎵 {pitchKey}
              </span>
            )}
          </div>

          {/* Saved Clips Section */}
          <div className="inspector-clips-section">
            <div className="inspector-subheading">
              <h3>Saved Clips</h3>
              <span className="badge-counter">{safeClips.length}</span>
            </div>

            {loadingClips ? (
              <small className="muted">Loading clips...</small>
            ) : safeClips.length === 0 ? (
              <p className="empty-clips-hint">
                No clip variants saved yet. Make a selection on the waveform and click <strong>Save Clip</strong>.
              </p>
            ) : (
              <div className="clips-list" role="list" aria-label="Saved clips">
                {safeClips.map((clip) => {
                  const rate =
                    clip.recipe.source_sample_rate_hz || p.sample_rate;
                  const startSec = frameToSeconds(
                    clip.recipe.start_frame,
                    rate,
                  );
                  const endSec = frameToSeconds(
                    clip.recipe.end_frame,
                    rate,
                  );
                  const clipDur = Math.max(0, endSec - startSec);

                  return (
                    <div
                      key={clip.id}
                      className={`clip-card ${clip.is_stale ? 'stale' : ''}`}
                      role="listitem"
                    >
                      <div className="clip-card-header">
                        <span className="clip-name" title={clip.name}>
                          <BookmarkCheck size={13} aria-hidden="true" />{' '}
                          {clip.name}
                        </span>
                        <div className="clip-badges">
                          <span className="clip-revision">
                            v{clip.revision}
                          </span>
                          <span className="clip-duration">
                            {duration(clipDur)}
                          </span>
                        </div>
                      </div>

                      <div className="clip-frames">
                        Frames: {clip.recipe.start_frame} –{' '}
                        {clip.recipe.end_frame}
                      </div>

                      {clip.is_stale && (
                        <div className="clip-stale-alert" role="alert">
                          <AlertTriangle size={13} aria-hidden="true" />
                          <span>Source changed (stale)</span>
                          <button
                            type="button"
                            className="compact-button rebind-button"
                            disabled={rebindingId === clip.id}
                            onClick={() => handleRebind(clip.id)}
                            title="Rebind clip boundaries to the updated audio file"
                          >
                            <RefreshCw
                              size={11}
                              className={
                                rebindingId === clip.id ? 'spinning' : ''
                              }
                            />
                            {rebindingId === clip.id
                              ? 'Rebinding...'
                              : 'Rebind'}
                          </button>
                        </div>
                      )}

                      <div className="clip-card-actions">
                        <button
                          type="button"
                          className="compact-button"
                          title="Load clip selection into waveform editor"
                          onClick={() => handleLoadClip(clip)}
                        >
                          Select
                        </button>
                        <button
                          type="button"
                          className="compact-button"
                          title={`Play clip region (${duration(clipDur)})`}
                          aria-label={`Play clip ${clip.name} (${duration(clipDur)})`}
                          disabled={clip.is_stale}
                          onClick={() => {
                            // Play only the clip region using the dedicated IPC command.
                            void playClip(sound.id, clip.id).catch(() => {});
                          }}
                        >
                          <Play size={11} aria-hidden="true" /> Play Clip
                        </button>
                        <button
                          type="button"
                          className="icon-button delete-clip-button"
                          title="Delete clip"
                          aria-label={`Delete clip ${clip.name}`}
                          onClick={() => handleDeleteClip(clip.id)}
                        >
                          <Trash2 size={13} aria-hidden="true" />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <h3>Measured profile</h3>
          <p className="description">{p.description}</p>
          <div className="tags">
            {p.tags.map((t) => (
              <span key={t} className="tag measured">
                {t.replaceAll('_', ' ')}
              </span>
            ))}
          </div>
        </>
      )}

      <h3>Your tags</h3>
      <div className="tags">
        {tags.map((t) => (
          <span className="tag" key={t}>
            {t}
            <button aria-label={`Remove tag ${t}`} onClick={() => removeTag(t)}>
              <X size={12} />
            </button>
          </span>
        ))}
      </div>

      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          addTag();
        }}
      >
        <Tag size={15} />
        <input
          aria-label="New tag"
          placeholder="Add a tag"
          maxLength={64}
          value={tag}
          onChange={(e) => setTag(e.target.value)}
        />
        <button
          className="icon-button"
          type="submit"
          title="Add tag"
          aria-label="Add tag"
        >
          <Check size={15} />
        </button>
      </form>

      <label className="comment-label">
        Comment
        <textarea
          aria-label="Comment"
          value={comment}
          maxLength={10000}
          onChange={(e) => {
            setComment(e.target.value);
            setSaved(false);
          }}
          rows={4}
        />
      </label>

      <button
        className="save-metadata"
        disabled={saving}
        onClick={handleSave}
      >
        <Check size={15} />
        {saving ? 'Saving...' : saved ? 'Saved' : 'Save changes'}
      </button>
    </aside>
  );
});
