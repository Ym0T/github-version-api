# 🚀 GitHub Version API

[![API Status](https://img.shields.io/badge/API-Online-brightgreen)](https://api.tavuru.de/health)
[![Response Time](https://img.shields.io/badge/Response%20Time-<100ms-brightgreen)](https://api.tavuru.de/stats)
[![P95 Latency](https://img.shields.io/badge/P95%20Latency-80ms-green)](https://api.tavuru.de/stats)
[![Reliability](https://img.shields.io/badge/Uptime-99.9%25-brightgreen)](https://api.tavuru.de/health)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange)](https://www.rust-lang.org/)

> **A blazing-fast, free public API for retrieving GitHub repository version information and version diffs. Built with Rust for maximum performance and reliability.**

## ✨ What is this?

This is a **free, public API service** that provides version information and version diffs for any GitHub repository. No API keys, no rate limits, no registration required. Just fast, reliable access to repository version data and change analysis.

🔗 **Base URL**: `https://api.tavuru.de`

## 📖 API Endpoints

| Endpoint | Method | Description | Example Response |
|----------|--------|-------------|------------------|
| **Core API** | | | |
| `/` | GET | API welcome message | Plain text status |
| `/health` | GET | Health check with uptime | JSON health data |
| `/stats` | GET | Enhanced usage statistics | JSON stats with cache details |
| **Version API** | | | |
| `/version/{owner}/{repo}` | GET | Get version info | JSON version data |
| `/version/{repo}` | GET | Get version (default owner) | JSON version data |
| `/version` | GET | Query params support | JSON version data |
| **🆕 Diff API** | | | |
| `/diff/{owner}/{repo}/{from}/{to}` | GET | Compare two versions | JSON diff data |
| `/diff/{repo}/{from}/{to}` | GET | Compare (default owner) | JSON diff data |
| **🆕 Download API** | | | |
| `/download/diff/{filename}` | GET | Download diff ZIP file | Binary ZIP file |

### 🆕 Diff API Query Parameters

| Parameter | Values | Description | Example |
|-----------|--------|-------------|---------|
| `zip` | `true`/`false` | Generate downloadable ZIP | `?zip=true` |
| `format` | `json` | Response format (default: json) | `?format=json` |

## 🎯 Why use this API?

- ⚡ **Lightning Fast** - Sub-100ms response times (69ms median)
- 🆓 **Completely Free** - No API keys, no rate limits, no costs
- 🌍 **Public Access** - Use in any project, personal or commercial
- 📊 **Rich Data** - Version info, file sizes, download counts, release notes
- 🆕 **Version Diffs** - Compare any two versions with detailed change analysis
- 📦 **ZIP Downloads** - Get changed files as downloadable ZIP archives
- 🛡️ **Rock Solid** - Memory-safe Rust implementation with 99.9% uptime
- 🔄 **Smart Caching** - Intelligent caching reduces GitHub API load
- ✅ **Proper Error Handling** - Clear HTTP status codes and error messages

## 🚀 Quick Start

### Version API Usage

```bash
# Get latest version of any GitHub repository
curl https://api.tavuru.de/version/microsoft/vscode

# Example response
{
  "version": "1.85.2",
  "release_name": "January 2024 Recovery",
  "published_at": "2024-01-22T10:30:00Z",
  "files": [
    {
      "name": "VSCode-win32-x64.zip",
      "size": 142857600,
      "size_formatted": "136.2 MB",
      "download_count": 1250000,
      "download_url": "https://github.com/microsoft/vscode/releases/download/1.85.2/VSCode-win32-x64.zip",
      "content_type": "application/zip"
    }
  ],
  "download_urls": {
    "zipball": "https://api.github.com/repos/microsoft/vscode/zipball/1.85.2",
    "tarball": "https://api.github.com/repos/microsoft/vscode/tarball/1.85.2"
  },
  "release_notes": "## What's New\n- Performance improvements\n- Bug fixes",
  "cached": true,
  "cache_expires_in": 563,
  "request_id": "abc123-def456-ghi789"
}
```

### 🆕 Diff API Usage

```bash
# Compare two versions of a repository
curl https://api.tavuru.de/diff/microsoft/vscode/1.85.0/1.85.2

# Generate a downloadable ZIP with changed files
curl "https://api.tavuru.de/diff/facebook/react/v18.2.0/v18.3.0?zip=true"

# Example diff response
{
  "from_version": "v18.2.0",
  "to_version": "v18.3.0",
  "repository": "facebook/react",
  "total_changes": 542,
  "files_changed": [
    {
      "filename": "packages/react/src/React.js",
      "status": "modified",
      "additions": 15,
      "deletions": 8,
      "changes": 23,
      "raw_url": "https://raw.githubusercontent.com/facebook/react/v18.3.0/packages/react/src/React.js",
      "previous_filename": null
    }
  ],
  "summary": {
    "total_additions": 287,
    "total_deletions": 255,
    "files_added": 12,
    "files_modified": 34,
    "files_removed": 5,
    "files_renamed": 2
  },
  "download_url": "https://api.tavuru.de/download/diff/diff-facebook-react-v18.2.0-v18.3.0-abc12345.zip",
  "zip_size": 2048576,
  "zip_size_formatted": "2.0 MB",
  "generated_at": "2025-06-23T12:00:00Z",
  "expires_at": "2025-06-24T12:00:00Z",
  "cached": false,
  "cache_expires_in": 604800,
  "request_id": "diff-abc123-def456"
}
```

### Supported URL Patterns

```bash
# Version API patterns
https://api.tavuru.de/version/facebook/react
https://api.tavuru.de/version/microsoft/typescript
https://api.tavuru.de/version/docker/compose

# Query parameters (legacy support)
https://api.tavuru.de/version?owner=torvalds&repo=linux

# Default owner shorthand (uses Ym0T as default)
https://api.tavuru.de/version/pterodactyl-nginx-egg

# 🆕 Diff API patterns
https://api.tavuru.de/diff/microsoft/vscode/1.85.0/1.85.2
https://api.tavuru.de/diff/pterodactyl-nginx-egg/v2.0/v2.1  # Uses default owner
https://api.tavuru.de/diff/facebook/react/v18.2.0/v18.3.0?zip=true  # With ZIP
```

## 💻 Code Examples

### 🆕 Version Diff Analysis

#### JavaScript/TypeScript

```javascript
// Get version diff with detailed analysis
async function getVersionDiff(owner, repo, fromVersion, toVersion, createZip = false) {
  try {
    const url = `https://api.tavuru.de/diff/${owner}/${repo}/${fromVersion}/${toVersion}`;
    const params = new URLSearchParams();
    if (createZip) params.append('zip', 'true');
    
    const response = await fetch(`${url}?${params}`);
    
    if (!response.ok) {
      const error = await response.json();
      throw new Error(`${error.error}: ${error.message}`);
    }
    
    return await response.json();
  } catch (error) {
    console.error('Failed to get diff:', error.message);
    return null;
  }
}

// Analyze changes between versions
async function analyzeChanges(owner, repo, fromVersion, toVersion) {
  const diff = await getVersionDiff(owner, repo, fromVersion, toVersion);
  
  if (!diff) return null;
  
  const analysis = {
    repository: diff.repository,
    versions: `${diff.from_version} → ${diff.to_version}`,
    totalFiles: diff.files_changed.length,
    totalChanges: diff.total_changes,
    breakdown: {
      added: diff.summary.files_added,
      modified: diff.summary.files_modified,
      removed: diff.summary.files_removed,
      renamed: diff.summary.files_renamed
    },
    impact: {
      additions: diff.summary.total_additions,
      deletions: diff.summary.total_deletions,
      netChange: diff.summary.total_additions - diff.summary.total_deletions
    },
    cached: diff.cached,
    cacheExpires: diff.cache_expires_in
  };
  
  return analysis;
}

// Usage example
const changes = await analyzeChanges('facebook', 'react', 'v18.2.0', 'v18.3.0');
if (changes) {
  console.log(`📊 Changes in ${changes.repository}:`);
  console.log(`   Versions: ${changes.versions}`);
  console.log(`   Files changed: ${changes.totalFiles}`);
  console.log(`   Total changes: ${changes.totalChanges}`);
  console.log(`   Net impact: +${changes.impact.netChange} lines`);
}

// Download diff as ZIP
async function downloadDiffZip(owner, repo, fromVersion, toVersion) {
  const diff = await getVersionDiff(owner, repo, fromVersion, toVersion, true);
  
  if (diff?.download_url) {
    // Create download link
    const link = document.createElement('a');
    link.href = diff.download_url;
    link.download = `${repo}-${fromVersion}-to-${toVersion}.zip`;
    link.click();
    
    console.log(`📦 ZIP downloaded: ${diff.zip_size_formatted}`);
    return true;
  }
  
  return false;
}
```

#### Python

```python
import requests
from typing import Optional, Dict, Any, List

def get_version_diff(owner: str, repo: str, from_version: str, to_version: str, 
                    create_zip: bool = False) -> Optional[Dict[str, Any]]:
    """Get detailed diff between two versions"""
    url = f"https://api.tavuru.de/diff/{owner}/{repo}/{from_version}/{to_version}"
    params = {'zip': 'true'} if create_zip else {}
    
    try:
        response = requests.get(url, params=params, timeout=30)
        
        if response.status_code == 404:
            print(f"Repository or versions not found: {owner}/{repo}")
            return None
        elif response.status_code == 400:
            error = response.json()
            print(f"Invalid request: {error['message']}")
            return None
        
        response.raise_for_status()
        return response.json()
        
    except requests.exceptions.Timeout:
        print("Request timed out")
        return None
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None

def analyze_project_changes(projects: List[tuple]) -> None:
    """Analyze changes across multiple projects"""
    print("🔍 Analyzing version changes...\n")
    
    for owner, repo, from_ver, to_ver in projects:
        print(f"📊 {repo}: {from_ver} → {to_ver}")
        
        diff = get_version_diff(owner, repo, from_ver, to_ver)
        
        if not diff:
            print("   ❌ Failed to get diff\n")
            continue
        
        summary = diff['summary']
        cached_info = "💾 (cached)" if diff['cached'] else "🌐 (fresh)"
        
        print(f"   Total files changed: {len(diff['files_changed'])}")
        print(f"   Changes breakdown:")
        print(f"     📝 Modified: {summary['files_modified']}")
        print(f"     ➕ Added: {summary['files_added']}")
        print(f"     ➖ Removed: {summary['files_removed']}")
        print(f"     🔄 Renamed: {summary['files_renamed']}")
        print(f"   Code changes: +{summary['total_additions']} -{summary['total_deletions']}")
        print(f"   {cached_info}")
        
        # Check if ZIP is available
        if diff.get('download_url'):
            print(f"   📦 ZIP available: {diff['zip_size_formatted']}")
        
        print()

def download_diff_zip(owner: str, repo: str, from_version: str, to_version: str, 
                     filename: Optional[str] = None) -> bool:
    """Download diff as ZIP file"""
    diff = get_version_diff(owner, repo, from_version, to_version, create_zip=True)
    
    if not diff or not diff.get('download_url'):
        print("❌ ZIP not available for this diff")
        return False
    
    try:
        zip_response = requests.get(diff['download_url'], timeout=60)
        zip_response.raise_for_status()
        
        if not filename:
            filename = f"{repo}-{from_version}-to-{to_version}.zip"
        
        with open(filename, 'wb') as f:
            f.write(zip_response.content)
        
        print(f"📦 Downloaded: {filename} ({diff['zip_size_formatted']})")
        return True
        
    except Exception as e:
        print(f"❌ Download failed: {e}")
        return False

# Usage examples
if __name__ == "__main__":
    # Analyze multiple projects
    projects = [
        ('microsoft', 'vscode', '1.85.0', '1.85.2'),
        ('facebook', 'react', 'v18.2.0', 'v18.3.0'),
        ('docker', 'compose', 'v2.24.0', 'v2.25.0'),
    ]
    
    analyze_project_changes(projects)
    
    # Download a specific diff
    print("📦 Downloading React diff...")
    download_diff_zip('facebook', 'react', 'v18.2.0', 'v18.3.0')
```

#### Bash/Shell

```bash
#!/bin/bash

# Function to get version diff
get_diff() {
    local owner="$1"
    local repo="$2"
    local from_ver="$3"
    local to_ver="$4"
    local create_zip="${5:-false}"
    
    local url="https://api.tavuru.de/diff/$owner/$repo/$from_ver/$to_ver"
    if [[ "$create_zip" == "true" ]]; then
        url="$url?zip=true"
    fi
    
    local response=$(curl -s -w "HTTPSTATUS:%{http_code}" "$url")
    local body=$(echo "$response" | sed -E 's/HTTPSTATUS:[0-9]{3}$//')
    local status=$(echo "$response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
    
    case "$status" in
        200)
            echo "$body"
            return 0
            ;;
        404)
            echo "❌ Repository or versions not found: $owner/$repo" >&2
            return 1
            ;;
        400)
            echo "❌ Invalid request parameters" >&2
            return 1
            ;;
        *)
            echo "❌ API error (HTTP $status)" >&2
            return 1
            ;;
    esac
}

# Analyze changes between versions
analyze_diff() {
    local owner="$1"
    local repo="$2"
    local from_ver="$3"
    local to_ver="$4"
    
    echo "🔍 Analyzing $repo: $from_ver → $to_ver"
    
    if diff_data=$(get_diff "$owner" "$repo" "$from_ver" "$to_ver"); then
        local files_changed=$(echo "$diff_data" | jq '.files_changed | length')
        local total_changes=$(echo "$diff_data" | jq '.total_changes')
        local added=$(echo "$diff_data" | jq '.summary.files_added')
        local modified=$(echo "$diff_data" | jq '.summary.files_modified')
        local removed=$(echo "$diff_data" | jq '.summary.files_removed')
        local additions=$(echo "$diff_data" | jq '.summary.total_additions')
        local deletions=$(echo "$diff_data" | jq '.summary.total_deletions')
        local cached=$(echo "$diff_data" | jq -r '.cached')
        
        echo "   📊 Files changed: $files_changed"
        echo "   📝 Modified: $modified, ➕ Added: $added, ➖ Removed: $removed"
        echo "   📈 Code changes: +$additions -$deletions"
        
        if [[ "$cached" == "true" ]]; then
            echo "   💾 (from cache)"
        else
            echo "   🌐 (fresh from GitHub)"
        fi
        
        return 0
    else
        echo "   ❌ Failed to get diff"
        return 1
    fi
}

# Download diff as ZIP
download_diff() {
    local owner="$1"
    local repo="$2"
    local from_ver="$3"
    local to_ver="$4"
    local filename="${repo}-${from_ver}-to-${to_ver}.zip"
    
    echo "📦 Generating ZIP for $repo diff..."
    
    if diff_data=$(get_diff "$owner" "$repo" "$from_ver" "$to_ver" "true"); then
        local download_url=$(echo "$diff_data" | jq -r '.download_url // empty')
        local zip_size=$(echo "$diff_data" | jq -r '.zip_size_formatted // empty')
        
        if [[ -n "$download_url" && "$download_url" != "null" ]]; then
            echo "⬇️  Downloading: $filename ($zip_size)"
            
            if curl -s -L -o "$filename" "$download_url"; then
                echo "✅ Downloaded: $filename"
                return 0
            else
                echo "❌ Download failed"
                return 1
            fi
        else
            echo "❌ ZIP not available (too many changes or generation failed)"
            return 1
        fi
    else
        echo "❌ Failed to generate diff"
        return 1
    fi
}

# Usage examples
echo "=== Version Diff Analysis ==="
analyze_diff "microsoft" "vscode" "1.85.0" "1.85.2"
analyze_diff "facebook" "react" "v18.2.0" "v18.3.0"
analyze_diff "docker" "compose" "v2.24.0" "v2.25.0"

echo -e "\n=== Download Example ==="
download_diff "facebook" "react" "v18.2.0" "v18.3.0"
```

### Version API Examples

#### JavaScript/TypeScript

```javascript
// Simple version check
async function getLatestVersion(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    if (!response.ok) {
      const error = await response.json();
      throw new Error(`${error.error}: ${error.message}`);
    }
    
    const data = await response.json();
    return data.version;
  } catch (error) {
    console.error('Failed to get version:', error.message);
    return null;
  }
}

// Usage with error handling
const version = await getLatestVersion('microsoft', 'vscode');
if (version) {
  console.log(`Latest VS Code: ${version}`);
} else {
  console.log('Repository not found or API error');
}

// Check for updates
async function checkForUpdate(currentVersion, owner, repo) {
  const latest = await getLatestVersion(owner, repo);
  return latest && latest !== currentVersion;
}

// Get complete repository information
async function getRepositoryInfo(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    if (response.status === 404) {
      return { error: 'Repository not found' };
    }
    
    if (!response.ok) {
      const error = await response.json();
      return { error: error.message };
    }
    
    return await response.json();
  } catch (error) {
    return { error: 'Network error' };
  }
}
```

## 📊 Response Formats

### 🆕 Diff Response Format

```typescript
interface DiffResponse {
  from_version: string;          // Source version
  to_version: string;            // Target version
  repository: string;            // Repository identifier
  total_changes: number;         // Total number of changes
  files_changed: FileChange[];   // Array of changed files
  summary: DiffSummary;          // Change summary
  download_url?: string;         // ZIP download URL (if requested)
  zip_size?: number;            // ZIP file size in bytes
  zip_size_formatted?: string;   // Human-readable ZIP size
  generated_at: string;          // ISO 8601 generation timestamp
  expires_at: string;            // ISO 8601 expiration timestamp
  cached: boolean;               // Whether response was cached
  cache_expires_in: number;      // Cache TTL in seconds
  request_id: string;            // Unique request identifier
}

interface FileChange {
  filename: string;              // File path
  status: string;                // "added", "modified", "removed", "renamed"
  additions: number;             // Lines added
  deletions: number;             // Lines deleted
  changes: number;               // Total changes (additions + deletions)
  raw_url?: string;             // Direct file URL
  previous_filename?: string;    // Original filename (for renamed files)
}

interface DiffSummary {
  total_additions: number;       // Total lines added
  total_deletions: number;       // Total lines deleted
  files_added: number;           // Number of new files
  files_modified: number;        // Number of modified files
  files_removed: number;         // Number of deleted files
  files_renamed: number;         // Number of renamed files
}
```

### Version Response Format

```typescript
interface VersionResponse {
  version: string;           // Git tag or version identifier
  release_name?: string;     // Human-readable release name  
  published_at: string;      // ISO 8601 timestamp
  files: FileInfo[];         // Array of downloadable files
  download_urls: {
    zipball: string;         // ZIP archive URL
    tarball: string;         // TAR.GZ archive URL
  };
  release_notes?: string;    // Markdown release notes
  cached: boolean;           // Whether response was cached
  cache_expires_in: number;  // Cache TTL in seconds
  request_id: string;        // Unique request identifier
}

interface FileInfo {
  name: string;              // File name
  size: number;              // Size in bytes
  size_formatted: string;    // Human-readable size (e.g., "1.2 MB")
  download_count: number;    // GitHub download count
  download_url: string;      // Direct download URL
  content_type?: string;     // MIME type
}
```

### Error Response

```typescript
interface ErrorResponse {
  error: string;             // Error code
  message: string;           // Human-readable error message
  request_id: string;        // Unique request identifier
  timestamp: string;         // ISO 8601 timestamp
}
```

### Common Error Codes

| HTTP Status | Error Code | Description |
|-------------|------------|-------------|
| `404` | `repository_not_found` | Repository doesn't exist or is private |
| `404` | `version_not_found` | One or both versions not found |
| `400` | `invalid_versions` | Invalid version parameters |
| `400` | `identical_versions` | From and to versions are the same |
| `413` | `too_many_changes` | Diff too large to process safely |
| `500` | `network_error` | GitHub API or network issues |
| `500` | `internal_server_error` | Unexpected server error |

## ⚡ Performance & Caching

This API is built for speed and reliability:

- **Response Time**: Sub-100ms average (69ms median)
- **95th Percentile**: 80ms - Consistently fast responses
- **Cache Strategy**: Intelligent multi-tier caching
  - **Version info**: 10 minutes (data changes frequently)
  - **Small diffs** (< 10 files): 7 days (changes rarely)
  - **Medium diffs** (10-50 files): 30 days
  - **Large diffs** (50+ files): 90 days
- **ZIP Generation**: On-demand with 24-hour cleanup
- **Uptime**: 99.9%+ target

## 🎨 Use Cases

### 🆕 With Diff API
- **Code Review Automation** - Analyze changes between versions automatically
- **Release Impact Analysis** - Understand what changed between releases
- **Dependency Change Tracking** - Monitor changes in your dependencies
- **Automated Testing** - Download changed files for targeted testing
- **Documentation Updates** - Track changes that might require doc updates
- **Security Audits** - Review security-related changes between versions

### For Developers
- **Dependency Monitoring** - Track updates for your project dependencies
- **CI/CD Integration** - Automate version checks in your build pipeline
- **Update Notifications** - Alert users when new versions are available
- **Dashboard Creation** - Display current versions across multiple projects

### For Applications  
- **Desktop Apps** - Check for application updates
- **Mobile Apps** - Implement update notifications
- **Web Services** - Monitor service dependencies
- **DevOps Tools** - Automate infrastructure updates

## 🔧 Data Sources

The API intelligently fetches information from multiple sources:

### Version Data
1. **GitHub Releases API** (primary) - Full release data with assets and download counts
2. **GitHub Tags API** (fallback) - Tag-based version info when no releases exist
3. **VERSION files** (fallback) - Repository version files in root directory
4. **package.json** (fallback) - Node.js package version information

### 🆕 Diff Data
1. **GitHub Compare API** - Detailed file-by-file comparison
2. **Smart Caching** - Long-term storage with complexity-based TTL
3. **On-demand ZIP Generation** - Download changed files as archives

## 🛡️ Fair Use Policy

This API is **completely free** with no rate limits. Please use responsibly:

- ✅ **Personal and commercial use** - No restrictions
- ✅ **No authentication required** - Just start using it
- ✅ **No rate limits** - Use as much as you need
- ✅ **Cache responses** - Client-side caching recommended for better performance
- ❌ **Don't abuse** - Please avoid excessive requests that could impact service

## 📊 API Status & Monitoring

Check API health and get real-time statistics:

```bash
# Health check
curl https://api.tavuru.de/health

# Enhanced usage statistics  
curl https://api.tavuru.de/stats

# Example enhanced stats response
{
  "total_requests": 45632,
  "cache_hits": 38291,
  "cache_misses": 7341,
  "cache_hit_rate": 83.89,
  "uptime": "15d 7h 23m 45s",
  "memory_usage": {
    "version_cache_entries": 1247,
    "diff_cache_entries": 3891,
    "total_cache_entries": 5138,
    "estimated_memory_usage": "5.1 MB",
    "cache_efficiency": 91.2
  },
  "cache_details": {
    "version_cache": {
      "entries": 1247,
      "hit_rate": 83.89,
      "average_ttl": "10 minutes",
      "description": "Caches repository version information"
    },
    "diff_cache": {
      "entries": 3891,
      "hit_rate": 89.15,
      "average_ttl": "7-90 days (based on diff complexity)",
      "description": "Long-term cache for version diffs (never change)"
    }
  },
  "performance_metrics": {
    "requests_per_second": 2.15,
    "average_response_time_estimate": "5-15ms (mostly cached)",
    "cache_effectiveness": "Excellent (>90% hit rate)"
  }
}
```

## 🚨 Error Handling Best Practices

When using this API, always implement proper error handling:

### JavaScript Example
```javascript
async function safeApiCall(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    // Handle different HTTP status codes
    switch (response.status) {
      case 200:
        return await response.json();
      case 404:
        const error = await response.json();
        throw new Error(`Repository not found: ${error.message}`);
      case 500:
        throw new Error('API server error, please try again later');
      default:
        throw new Error(`Unexpected status: ${response.status}`);
    }
  } catch (error) {
    console.error('API call failed:', error.message);
    return null;
  }
}

// 🆕 Diff API error handling
async function safeDiffCall(owner, repo, fromVersion, toVersion) {
  try {
    const response = await fetch(`https://api.tavuru.de/diff/${owner}/${repo}/${fromVersion}/${toVersion}`);
    
    switch (response.status) {
      case 200:
        return await response.json();
      case 404:
        const error = await response.json();
        if (error.error === 'version_not_found') {
          throw new Error(`Version not found: ${error.message}`);
        } else {
          throw new Error(`Repository not found: ${error.message}`);
        }
      case 400:
        const badRequest = await response.json();
        throw new Error(`Invalid request: ${badRequest.message}`);
      case 413:
        throw new Error('Diff too large to process - try smaller version ranges');
      case 500:
        throw new Error('API server error, please try again later');
      default:
        throw new Error(`Unexpected status: ${response.status}`);
    }
  } catch (error) {
    console.error('Diff API call failed:', error.message);
    return null;
  }
}
```

### Python Example
```python
import requests

def safe_api_call(owner, repo):
    try:
        response = requests.get(
            f"https://api.tavuru.de/version/{owner}/{repo}",
            timeout=10
        )
        
        if response.status_code == 200:
            return response.json()
        elif response.status_code == 404:
            print(f"Repository {owner}/{repo} not found")
            return None
        else:
            print(f"API error: {response.status_code}")
            return None
            
    except requests.exceptions.Timeout:
        print("Request timed out")
        return None
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None

# 🆕 Diff API error handling
def safe_diff_call(owner, repo, from_version, to_version, create_zip=False):
    try:
        url = f"https://api.tavuru.de/diff/{owner}/{repo}/{from_version}/{to_version}"
        params = {'zip': 'true'} if create_zip else {}
        
        response = requests.get(url, params=params, timeout=30)
        
        if response.status_code == 200:
            return response.json()
        elif response.status_code == 404:
            error = response.json()
            if error['error'] == 'version_not_found':
                print(f"Version not found: {error['message']}")
            else:
                print(f"Repository not found: {error['message']}")
            return None
        elif response.status_code == 400:
            error = response.json()
            print(f"Invalid request: {error['message']}")
            return None
        elif response.status_code == 413:
            print("Diff too large to process - try smaller version ranges")
            return None
        else:
            print(f"API error: {response.status_code}")
            return None
            
    except requests.exceptions.Timeout:
        print("Request timed out (diffs can take longer)")
        return None
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None
```

## 🆕 Advanced Diff Features

### ZIP Archive Contents

When you request a diff with `?zip=true`, the generated ZIP file contains:

1. **README.md** - Comprehensive diff summary with metadata
2. **Changed Files** - All modified/added files from the target version
3. **Error Files** - Info about files that couldn't be downloaded
4. **Organized Structure** - Maintains original directory structure

### ZIP Safety Limits

For security and performance, ZIP generation has these limits:
- **Maximum 100 files** - Prevents excessive processing
- **Maximum 10MB per file** - Prevents memory issues
- **Maximum 10,000 total changes** - Ensures reasonable processing time
- **24-hour auto-cleanup** - Files are automatically removed

### Cache Strategy

The Diff API uses intelligent caching based on complexity:

| Diff Size | Cache Duration | Reasoning |
|-----------|----------------|-----------|
| **Small** (< 10 files) | 7 days | Quick to process, moderate caching |
| **Medium** (10-50 files) | 30 days | More expensive, longer cache |
| **Large** (50+ files) | 90 days | Very expensive, maximum cache |

> **Why long caching?** Version diffs never change - the difference between v1.0 and v1.1 will always be the same, so aggressive caching improves performance.

## 🔧 GitHub Actions Integration

### Automated Dependency Monitoring

```yaml
name: Monitor Dependencies with Diff Analysis
on:
  schedule:
    - cron: '0 9 * * MON'  # Every Monday at 9 AM
  workflow_dispatch:

jobs:
  check-updates:
    runs-on: ubuntu-latest
    steps:
      - name: Check VS Code Updates with Diff
        id: vscode
        run: |
          # Get current version
          CURRENT="1.85.1"
          LATEST=$(curl -s "https://api.tavuru.de/version/microsoft/vscode" | jq -r '.version')
          
          if [ "$LATEST" != "$CURRENT" ]; then
            echo "update_available=true" >> $GITHUB_OUTPUT
            echo "current=$CURRENT" >> $GITHUB_OUTPUT
            echo "latest=$LATEST" >> $GITHUB_OUTPUT
            
            # Get detailed diff
            DIFF=$(curl -s "https://api.tavuru.de/diff/microsoft/vscode/$CURRENT/$LATEST")
            FILES_CHANGED=$(echo "$DIFF" | jq '.files_changed | length')
            TOTAL_CHANGES=$(echo "$DIFF" | jq '.total_changes')
            
            echo "files_changed=$FILES_CHANGED" >> $GITHUB_OUTPUT
            echo "total_changes=$TOTAL_CHANGES" >> $GITHUB_OUTPUT
            
            echo "🆕 VS Code update: $CURRENT → $LATEST"
            echo "📊 Changes: $FILES_CHANGED files, $TOTAL_CHANGES total changes"
          else
            echo "update_available=false" >> $GITHUB_OUTPUT
            echo "✅ VS Code is up to date: $CURRENT"
          fi
      
      - name: Create Detailed Update Issue
        if: steps.vscode.outputs.update_available == 'true'
        uses: actions/github-script@v7
        with:
          script: |
            // Get detailed diff information
            const diffResponse = await fetch(
              `https://api.tavuru.de/diff/microsoft/vscode/${{ steps.vscode.outputs.current }}/${{ steps.vscode.outputs.latest }}`
            );
            const diffData = await diffResponse.json();
            
            // Analyze file types
            const fileTypes = {};
            diffData.files_changed.forEach(file => {
              const ext = file.filename.split('.').pop();
              fileTypes[ext] = (fileTypes[ext] || 0) + 1;
            });
            
            const fileTypesList = Object.entries(fileTypes)
              .sort(([,a], [,b]) => b - a)
              .slice(0, 5)
              .map(([ext, count]) => `- **.${ext}**: ${count} files`)
              .join('\n');
            
            const issueBody = `## 🆕 VS Code Update Available
            
            **Version Change:** \`${{ steps.vscode.outputs.current }}\` → \`${{ steps.vscode.outputs.latest }}\`
            
            ### 📊 Change Summary
            - **Files Changed:** ${{ steps.vscode.outputs.files_changed }}
            - **Total Changes:** ${{ steps.vscode.outputs.total_changes }}
            - **Files Added:** ${diffData.summary.files_added}
            - **Files Modified:** ${diffData.summary.files_modified}
            - **Files Removed:** ${diffData.summary.files_removed}
            - **Code Changes:** +${diffData.summary.total_additions} -${diffData.summary.total_deletions}
            
            ### 📁 Most Changed File Types
            ${fileTypesList}
            
            ### 🔗 Quick Links
            - [View Full Diff](https://api.tavuru.de/diff/microsoft/vscode/${{ steps.vscode.outputs.current }}/${{ steps.vscode.outputs.latest }})
            - [Download Changes ZIP](https://api.tavuru.de/diff/microsoft/vscode/${{ steps.vscode.outputs.current }}/${{ steps.vscode.outputs.latest }}?zip=true)
            - [GitHub Release](https://github.com/microsoft/vscode/releases/tag/${{ steps.vscode.outputs.latest }})
            
            ### 🤖 Next Steps
            - [ ] Review the changes
            - [ ] Test with the new version
            - [ ] Update dependency version
            - [ ] Close this issue when complete
            
            ---
            *Automatically generated by GitHub Actions using [GitHub Version API](https://api.tavuru.de)*`;
            
            github.rest.issues.create({
              owner: context.repo.owner,
              repo: context.repo.repo,
              title: `🆕 VS Code Update: ${{ steps.vscode.outputs.current }} → ${{ steps.vscode.outputs.latest }}`,
              body: issueBody,
              labels: ['dependencies', 'update-available', 'automated']
            });
```

## 🛠️ CLI Tools & Scripts

### Bash Script for Project Monitoring

```bash
#!/bin/bash

# project-monitor.sh - Monitor multiple projects for updates
set -euo pipefail

# Configuration file format: owner/repo:current_version
CONFIG_FILE="$HOME/.project-versions"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to check single project
check_project() {
    local project="$1"
    local current="$2"
    local owner=$(echo "$project" | cut -d'/' -f1)
    local repo=$(echo "$project" | cut -d'/' -f2)
    
    echo -e "${BLUE}🔍 Checking $project...${NC}"
    
    # Get latest version
    local latest_response=$(curl -s -w "HTTPSTATUS:%{http_code}" \
        "https://api.tavuru.de/version/$owner/$repo")
    local latest_body=$(echo "$latest_response" | sed -E 's/HTTPSTATUS:[0-9]{3}$//')
    local latest_status=$(echo "$latest_response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
    
    if [[ "$latest_status" != "200" ]]; then
        echo -e "   ${RED}❌ Failed to get version info${NC}"
        return 1
    fi
    
    local latest=$(echo "$latest_body" | jq -r '.version')
    local cached=$(echo "$latest_body" | jq -r '.cached')
    local cache_info=""
    
    if [[ "$cached" == "true" ]]; then
        cache_info=" ${YELLOW}(cached)${NC}"
    fi
    
    if [[ "$latest" == "$current" ]]; then
        echo -e "   ${GREEN}✅ Up to date: $current${NC}$cache_info"
        return 0
    fi
    
    echo -e "   ${YELLOW}🆕 Update available: $current → $latest${NC}$cache_info"
    
    # Get diff if update available
    local diff_response=$(curl -s -w "HTTPSTATUS:%{http_code}" \
        "https://api.tavuru.de/diff/$owner/$repo/$current/$latest")
    local diff_body=$(echo "$diff_response" | sed -E 's/HTTPSTATUS:[0-9]{3}$//')
    local diff_status=$(echo "$diff_response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
    
    if [[ "$diff_status" == "200" ]]; then
        local files_changed=$(echo "$diff_body" | jq '.files_changed | length')
        local total_changes=$(echo "$diff_body" | jq '.total_changes')
        local additions=$(echo "$diff_body" | jq '.summary.total_additions')
        local deletions=$(echo "$diff_body" | jq '.summary.total_deletions')
        local diff_cached=$(echo "$diff_body" | jq -r '.cached')
        
        local diff_cache_info=""
        if [[ "$diff_cached" == "true" ]]; then
            diff_cache_info=" (cached)"
        fi
        
        echo -e "   ${BLUE}📊 Changes: $files_changed files, $total_changes total (+$additions -$deletions)${NC}$diff_cache_info"
        
        # Offer to download ZIP
        read -p "   📦 Download changes as ZIP? [y/N]: " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            local zip_filename="${repo}-${current}-to-${latest}.zip"
            echo -e "   ${BLUE}⬇️  Downloading ZIP...${NC}"
            
            local zip_response=$(curl -s -w "HTTPSTATUS:%{http_code}" \
                "https://api.tavuru.de/diff/$owner/$repo/$current/$latest?zip=true")
            local zip_body=$(echo "$zip_response" | sed -E 's/HTTPSTATUS:[0-9]{3}$//')
            local zip_status=$(echo "$zip_response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
            
            if [[ "$zip_status" == "200" ]]; then
                local download_url=$(echo "$zip_body" | jq -r '.download_url // empty')
                local zip_size=$(echo "$zip_body" | jq -r '.zip_size_formatted // empty')
                
                if [[ -n "$download_url" && "$download_url" != "null" ]]; then
                    if curl -s -L -o "$zip_filename" "$download_url"; then
                        echo -e "   ${GREEN}✅ Downloaded: $zip_filename ($zip_size)${NC}"
                    else
                        echo -e "   ${RED}❌ Download failed${NC}"
                    fi
                else
                    echo -e "   ${YELLOW}⚠️  ZIP not available (too many changes)${NC}"
                fi
            else
                echo -e "   ${RED}❌ Failed to generate ZIP${NC}"
            fi
        fi
    else
        echo -e "   ${YELLOW}⚠️  Could not get diff information${NC}"
    fi
    
    return 0
}

# Main execution
main() {
    if [[ ! -f "$CONFIG_FILE" ]]; then
        echo -e "${YELLOW}📝 Creating config file: $CONFIG_FILE${NC}"
        cat > "$CONFIG_FILE" << EOF
# Project monitoring configuration
# Format: owner/repo:current_version
microsoft/vscode:1.85.1
facebook/react:18.2.0
docker/compose:2.24.0
# Add your projects here...
EOF
        echo -e "${GREEN}✅ Config file created. Edit $CONFIG_FILE to add your projects.${NC}"
        exit 0
    fi
    
    echo -e "${BLUE}🚀 Starting project update check...${NC}\n"
    
    local total=0
    local updates=0
    
    # Read config file
    while IFS=':' read -r project current || [[ -n "$project" ]]; do
        # Skip comments and empty lines
        [[ "$project" =~ ^[[:space:]]*# ]] && continue
        [[ -z "$project" ]] && continue
        
        ((total++))
        
        if check_project "$project" "$current"; then
            if [[ $? -eq 1 ]]; then  # Update available
                ((updates++))
            fi
        fi
        
        echo  # Empty line between projects
    done < "$CONFIG_FILE"
    
    echo -e "${BLUE}📊 Summary: Checked $total projects, $updates updates available${NC}"
    
    if [[ $updates -gt 0 ]]; then
        echo -e "${YELLOW}💡 Tip: Update your $CONFIG_FILE with new versions after upgrading${NC}"
    fi
}

# Show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Monitor multiple GitHub projects for version updates with detailed diff analysis.

OPTIONS:
    -h, --help    Show this help message
    
CONFIG FILE: $CONFIG_FILE
Format: owner/repo:current_version

Examples:
    microsoft/vscode:1.85.1
    facebook/react:18.2.0
    docker/compose:2.24.0

Features:
    ✅ Version checking with cache status
    📊 Detailed change analysis (files, additions, deletions)
    📦 Optional ZIP download of changes
    🎨 Colorized output with status indicators

Powered by: https://api.tavuru.de
EOF
}

# Handle arguments
case "${1:-}" in
    -h|--help)
        show_help
        exit 0
        ;;
    *)
        main "$@"
        ;;
esac
```

## 🤝 Support & Feedback

Found a bug? Have a feature request? Want to contribute?

- 🐛 **Report Issues**: [Open a GitHub Issue](../../issues/new)
- 💡 **Feature Requests**: [Start a Discussion](../../discussions/new)
- 📧 **Contact**: Create an issue for any questions
- ⭐ **Star this repo** if the API helped you!

## 🏗️ Technical Details

### Built With
- **[Rust](https://www.rust-lang.org/)** - Memory-safe systems programming
- **[Axum](https://github.com/tokio-rs/axum)** - High-performance web framework
- **[Tokio](https://tokio.rs/)** - Asynchronous runtime
- **[Reqwest](https://github.com/seanmonstar/reqwest)** - HTTP client
- **[Serde](https://serde.rs/)** - JSON serialization
- **[Moka](https://github.com/moka-rs/moka)** - High-performance caching
- **[ZIP](https://github.com/zip-rs/zip)** - ZIP archive generation
- **[Cloudflare](https://cloudflare.com/)** - Global CDN and DDoS protection

### Architecture
- **Single-core optimization** - Maximum efficiency with minimal resources
- **Multi-tier caching** - Different TTLs for different data types
- **Graceful degradation** - Multiple fallback data sources
- **Zero downtime deployment** - Built for reliability
- **Proper error handling** - Clear HTTP status codes and error messages
- **Memory safety** - Rust prevents common security vulnerabilities

### Security
- **HTTPS only** - All communications encrypted
- **No data collection** - Privacy-focused design
- **Input validation** - Prevents malicious requests
- **Path traversal protection** - Safe file handling
- **Rate limiting friendly** - Smart caching reduces GitHub API load
- **Automatic cleanup** - Files removed after 24 hours

### 🆕 Diff API Architecture

- **GitHub Compare API Integration** - Direct GitHub API usage for accuracy
- **Intelligent Caching Strategy** - Complexity-based TTL (7-90 days)
- **On-demand ZIP Generation** - Files created only when requested
- **Safety Limits** - Prevents abuse with file/size/change limits
- **Memory Optimization** - Compact storage for long-term caching
- **Automatic Cleanup** - ZIP files removed after 24 hours

## 🔗 Quick Links

- **🌐 API Base URL**: https://api.tavuru.de
- **💚 Health Check**: https://api.tavuru.de/health  
- **📊 Statistics**: https://api.tavuru.de/stats
- **🧪 Test Version API**: `curl https://api.tavuru.de/version/microsoft/vscode`
- **🆕 Test Diff API**: `curl https://api.tavuru.de/diff/facebook/react/v18.2.0/v18.3.0`
- **📦 Test ZIP Download**: `curl "https://api.tavuru.de/diff/docker/compose/v2.24.0/v2.25.0?zip=true"`

---

<div align="center">

**Made with ❤️ and ⚡ by [Ym0T](https://github.com/Ym0T)**

*Built with Rust for blazing-fast performance • Powered by Cloudflare for global reliability*

**If this API saved you time, consider ⭐ starring this repository!**

</div>
